//! Map visible insertion cells before native drag-source adjacency checks.
use windows_sys::Win32::{
    Foundation::{POINT, RECT},
    Graphics::Gdi::{ClientToScreen, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTONEAREST, MonitorFromPoint},
    UI::{Controls::{LVM_GETITEMPOSITION, LVM_GETITEMSPACING}, WindowsAndMessaging::SendMessageW},
};

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
