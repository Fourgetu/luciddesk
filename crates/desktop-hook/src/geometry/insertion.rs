//! Map visible insertion cells before native drag-source adjacency checks.
use windows_sys::Win32::{
    Foundation::{POINT, RECT},
    Graphics::Gdi::{ClientToScreen, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTONEAREST, MonitorFromPoint},
    UI::{Controls::{LVINSERTMARK, LVIM_AFTER, LVM_GETITEMPOSITION, LVM_GETITEMSPACING}, WindowsAndMessaging::SendMessageW},
};

struct Entry {
    item: i32,
    native: POINT,
    display: POINT,
    hidden: bool,
    monitor: isize,
}

/// One ordering model for the native insertion identity and its compact marker.
struct VisibleOrder(Vec<Entry>);

impl VisibleOrder {
    fn boundary(&self, item: i32, after: bool) -> Option<(Vec<&Entry>, usize)> {
        let monitor = self.0.iter().find(|entry| entry.item == item)?.monitor;
        let mut entries: Vec<_> = self.0.iter().filter(|entry| entry.monitor == monitor).collect();
        entries.sort_by_key(|entry| (entry.native.x, entry.native.y));
        let position = entries.iter().position(|entry| entry.item == item)? + usize::from(after);
        let rank = entries[..position].iter().filter(|entry| !entry.hidden).count();
        Some((entries, rank))
    }

    fn canonical(&self, item: i32, after: bool) -> Option<(i32, bool)> {
        let (entries, rank) = self.boundary(item, after)?;
        if !entries.iter().any(|entry| !entry.hidden) { return None; }
        if let Some(next) = entries.iter().filter(|entry| !entry.hidden).nth(rank) {
            Some((next.item, false))
        } else {
            // Shell turns "after item" into item + 1. Use the physical end of
            // this monitor's native order so a hidden tail cannot be the target.
            entries.last().map(|last| (last.item, true))
        }
    }

    fn marker_anchor(&self, item: i32, after: bool) -> Option<POINT> {
        let (entries, rank) = self.boundary(item, after)?;
        if after {
            // The canonical end is after the last native item, even if hidden.
            // Render it after the last visible item in the same order.
            if rank != entries.iter().filter(|entry| !entry.hidden).count() { return None; }
            entries.iter().rev().find(|entry| !entry.hidden).map(|entry| entry.display)
        } else {
            entries.get(rank).map(|entry| entry.native)
        }
    }
}

fn visible_order() -> Option<VisibleOrder> {
    let (view, items) = super::STATE.with(|s| {
        let state = s.borrow();
        let state = state.as_ref()?;
        if !state.identities.active || !state.panes.is_empty() { return None; }
        Some((state.view, state.targets.iter().map(|(&i, &p)| (i, p)).collect::<Vec<_>>()))
    })?;
    let mut origin = POINT::default();
    if unsafe { ClientToScreen(view, &raw mut origin) } == 0 { return None; }
    let mut entries = Vec::with_capacity(items.len());
    for (item, display) in items {
        let mut native = POINT::default();
        if super::bypass(|| unsafe {
            SendMessageW(view, LVM_GETITEMPOSITION, usize::try_from(item).unwrap_or(usize::MAX), (&raw mut native) as isize)
        }) == 0 { return None; }
        let monitor = unsafe { MonitorFromPoint(POINT {
            x: native.x.saturating_add(origin.x), y: native.y.saturating_add(origin.y),
        }, MONITOR_DEFAULTTONEAREST) } as isize;
        entries.push(Entry { item, native, display, hidden: super::hidden::is_hidden(item), monitor });
    }
    Some(VisibleOrder(entries))
}

/// Canonicalize only boundaries that touch hidden identities. Folder hits and
/// native no-op marks (-1) are untouched, as are all ordinary visible gaps.
pub(super) fn normalize_mark(mark: &mut LVINSERTMARK) -> bool {
    if mark.iItem < 0 { return false; }
    let after = mark.dwFlags & LVIM_AFTER != 0;
    if !(super::hidden::is_hidden(mark.iItem)
        || after && super::hidden::is_hidden(mark.iItem.saturating_add(1))) { return false; }
    let Some((item, after)) = visible_order().and_then(|order| order.canonical(mark.iItem, after)) else { return false; };
    let flags = (mark.dwFlags & !LVIM_AFTER) | if after { LVIM_AFTER } else { 0 };
    let changed = (mark.iItem, mark.dwFlags) != (item, flags);
    mark.iItem = item;
    mark.dwFlags = flags;
    changed
}

/// Shell can canonicalize "after visible item" into "before next hidden item".
/// Keep that native insertion identity, but place its marker at the equivalent
/// boundary in the compact grid. Never move an item or rewrite Shell's mark.
pub(super) fn hidden_marker_anchor(item: i32, after: bool) -> Option<POINT> {
    if !super::hidden::is_hidden(item) { return None; }
    visible_order()?.marker_anchor(item, after)
}

pub(super) fn native_point(point: POINT) -> Option<POINT> {
    let view = super::STATE.with(|s| s.borrow().as_ref()
        .filter(|s| s.identities.active && s.panes.is_empty()).map(|s| s.view))?;
    let mut origin = POINT::default();
    if unsafe { ClientToScreen(view, &raw mut origin) } == 0 { return None; }
    let monitor = unsafe { MonitorFromPoint(POINT {
        x: point.x.saturating_add(origin.x), y: point.y.saturating_add(origin.y),
    }, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if unsafe { GetMonitorInfoW(monitor, &raw mut info) } == 0 { return None; }
    let local_monitor = RECT {
        left: info.rcMonitor.left - origin.x, top: info.rcMonitor.top - origin.y,
        right: info.rcMonitor.right - origin.x, bottom: info.rcMonitor.bottom - origin.y,
    };
    let cells: Vec<_> = super::STATE.with(|s| s.borrow().as_ref().map_or_else(Vec::new, |s| {
        s.targets.iter().filter(|(i,p)| !super::hidden::is_hidden(**i)
            && super::contains(&local_monitor, p.x, p.y)).map(|(&i,&p)| (i,p)).collect()
    }));
    let packed = u32::try_from(unsafe { SendMessageW(view, LVM_GETITEMSPACING, 0, 0) }).ok()?;
    let spacing = (i32::try_from(packed & 0xffff).ok()?, i32::try_from(packed >> 16).ok()?);
    let (item, offset) = cell_offset(point, &cells, spacing)?;
    let mut native = POINT::default();
    if super::bypass(|| unsafe { SendMessageW(view, LVM_GETITEMPOSITION,
        usize::try_from(item).unwrap_or(usize::MAX), (&raw mut native) as isize) }) == 0 { return None; }
    Some(POINT { x: native.x.saturating_add(offset.x), y: native.y.saturating_add(offset.y) })
}

fn cell_offset(point: POINT, cells: &[(i32, POINT)], spacing: (i32,i32)) -> Option<(i32,POINT)> {
    if spacing.0 <= 0 || spacing.1 <= 0 { return None; }
    let &(last, end) = cells.iter().max_by_key(|(_,p)| (p.x,p.y))?;
    if i64::from(point.x) >= i64::from(end.x) + i64::from(spacing.0)
        || (point.x >= end.x && i64::from(point.y) >= i64::from(end.y) + i64::from(spacing.1)) {
        return Some((last, POINT { x: spacing.0/2, y: spacing.1-1 }));
    }
    let &(item, cell) = cells.iter().min_by_key(|(_,p)| {
        let dx = i64::from(point.x) - i64::from(p.x) - i64::from(spacing.0/2);
        let dy = i64::from(point.y) - i64::from(p.y) - i64::from(spacing.1/2);
        dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
    })?;
    Some((item, POINT {
        x: point.x.saturating_sub(cell.x).clamp(0,spacing.0-1),
        y: point.y.saturating_sub(cell.y).clamp(0,spacing.1-1),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn order(hidden: u32) -> VisibleOrder {
        let mut rank = 0;
        VisibleOrder((0..5).map(|item| {
            let hidden = hidden & (1 << item) != 0;
            let display = POINT { x: 0, y: rank * 100 };
            if !hidden { rank += 1; }
            Entry { item, native: POINT { x: 0, y: item * 100 }, display, hidden, monitor: 1 }
        }).collect())
    }

    #[test]
    fn every_hidden_pattern_and_drag_selection_preserves_visible_drop_order() {
        // Exhaust all five-item hidden/selection sets, including multiple
        // selected sources, adjacent hidden runs, and a fully hidden tail.
        for hidden in 1..31 {
            let order = order(hidden);
            let visible: Vec<_> = order.0.iter().filter(|e| !e.hidden).map(|e| e.item).collect();
            for selected in 1..32 {
                if selected & hidden != 0 { continue; }
                let source: Vec<_> = visible.iter().copied().filter(|i| selected & (1 << i) != 0).collect();
                for item in 0..5 {
                    for after in [false, true] {
                        let (_, rank) = order.boundary(item, after).unwrap();
                        let (anchor, after) = order.canonical(item, after).unwrap();
                        assert!(after || !order.0[usize::try_from(anchor).unwrap()].hidden);
                        let boundary = usize::try_from(anchor).unwrap() + usize::from(after);
                        let native: Vec<_> = (0..5).collect();
                        let insertion = native[..boundary].iter().filter(|i| !source.contains(i)).count();
                        let mut actual: Vec<_> = native.into_iter().filter(|i| !source.contains(i)).collect();
                        actual.splice(insertion..insertion, source.iter().copied());
                        actual.retain(|i| hidden & (1 << i) == 0);
                        let insertion = visible[..rank].iter().filter(|i| !source.contains(i)).count();
                        let mut expected: Vec<_> = visible.iter().copied().filter(|i| !source.contains(i)).collect();
                        expected.splice(insertion..insertion, source.iter().copied());
                        assert_eq!(actual, expected, "hidden={hidden} selected={selected} boundary={rank}");
                    }
                }
            }
        }
    }

    #[test]
    fn hidden_end_and_other_monitor_do_not_become_visible_drop_targets() {
        let mut layout = order(0b11000);
        layout.0.push(Entry { item: 5, native: POINT { x: 200, y: 0 }, display: POINT { x: 200, y: 0 }, hidden: false, monitor: 2 });
        assert_eq!(layout.canonical(2, true), Some((4, true)));
        assert_eq!(layout.canonical(3, false), Some((4, true)));
        assert_eq!(layout.marker_anchor(4, true).unwrap().y, 200);
        assert_eq!(layout.canonical(5, false), Some((5, false)));
        assert_eq!(order(31).canonical(4, true), None);
    }
    fn cells() -> Vec<(i32,POINT)> {
        (0..81).filter(|i| *i != 57 && *i != 58).enumerate().map(|(rank,i)| {
            let rank = i32::try_from(rank).unwrap();
            (i, POINT { x: rank/14*112, y: 5+rank%14*147 })
        }).collect()
    }
    #[test]
    fn each_visible_gap_resolves_before_native_source_suppression() {
        let cells = cells();
        for &(item,p) in &cells {
            let (before,top) = cell_offset(POINT{x:p.x+56,y:p.y+10},&cells,(112,147)).unwrap();
            let (after,bottom) = cell_offset(POINT{x:p.x+56,y:p.y+137},&cells,(112,147)).unwrap();
            assert_eq!((before,after),(item,item));
            assert!(top.y < 147/2 && bottom.y > 147/2);
        }
    }
    #[test]
    fn recorded_tail_and_column_wrap_anchor_visible_identity() {
        let cells = cells();
        for p in [POINT{x:637,y:1336},POINT{x:634,y:1343},POINT{x:900,y:300}] {
            let (item,offset)=cell_offset(p,&cells,(112,147)).unwrap();
            assert_eq!((item,offset.y),(80,146));
        }
        // Last two native identities may themselves be collected.
        let cells:Vec<_>=cells.into_iter().filter(|(i,_)|*i<79).collect();
        assert_eq!(cell_offset(POINT{x:637,y:1336},&cells,(112,147)).unwrap().0,78);
    }
}
