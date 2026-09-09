//! UI-thread geometry caches. Never hold STATE across native control calls.
use super::{STATE, contains};
use std::{collections::{BTreeMap, BTreeSet, HashMap}, rc::Rc};
use windows_sys::Win32::{Foundation::{HWND, POINT, RECT}, UI::{Controls::{LVIR_ICON, LVIR_LABEL, LVM_GETITEMRECT}, WindowsAndMessaging::SendMessageW}};

#[derive(Default)]
pub(super) struct Cache {
    pub revision: u64,
    pub order: Option<Rc<super::insertion::VisibleOrder>>,
    hits: Option<Rc<Spatial<Hit>>>,
    damage: Option<Rc<Spatial<Damage>>>,
    bounds: BTreeMap<i32, (RECT, POINT)>,
}
impl Cache {
    pub fn layout_changed(&mut self) {
        self.order = None;
        self.geometry_changed(None);
    }
    pub fn geometry_changed(&mut self, item: Option<i32>) {
        self.revision = self.revision.wrapping_add(1);
        self.hits = None;
        self.damage = None;
        if let Some(item) = item { self.bounds.remove(&item); } else { self.bounds.clear(); }
    }
}

pub(super) fn layout_changed() {
    STATE.with(|s| { if let Some(s) = s.borrow_mut().as_mut() { s.cache.layout_changed(); } });
}
pub(super) fn geometry_changed(item: Option<i32>) {
    STATE.with(|s| { if let Some(s) = s.borrow_mut().as_mut() { s.cache.geometry_changed(item); } });
}

const CELL: i32 = 128;
struct Spatial<T> {
    entries: Vec<T>,
    buckets: HashMap<(i32, i32), Vec<usize>>,
    broad: Vec<usize>,
}
impl<T> Spatial<T> {
    fn new() -> Self { Self { entries: Vec::new(), buckets: HashMap::new(), broad: Vec::new() } }
    fn add(&mut self, entry: T, rects: &[RECT]) {
        let index = self.entries.len();
        self.entries.push(entry);
        for rect in rects {
            if rect.right <= rect.left || rect.bottom <= rect.top { continue; }
            let (l, t, r, b) = tiles(rect);
            if i64::from(r - l + 1) * i64::from(b - t + 1) > 4096 {
                self.broad.push(index);
                continue;
            }
            for x in l..=r { for y in t..=b { self.buckets.entry((x, y)).or_default().push(index); } }
        }
    }
    fn candidates(&self, rect: &RECT) -> Vec<usize> {
        let (l, t, r, b) = tiles(rect);
        if i64::from(r - l + 1) * i64::from(b - t + 1) > 4096 { return (0..self.entries.len()).collect(); }
        let mut indices: BTreeSet<_> = self.broad.iter().copied().collect();
        for x in l..=r { for y in t..=b {
            if let Some(bucket) = self.buckets.get(&(x, y)) { indices.extend(bucket); }
        } }
        indices.into_iter().collect()
    }
}
fn tiles(rect: &RECT) -> (i32, i32, i32, i32) {
    (rect.left.div_euclid(CELL), rect.top.div_euclid(CELL),
     rect.right.saturating_sub(1).div_euclid(CELL), rect.bottom.saturating_sub(1).div_euclid(CELL))
}

struct Hit { item: i32, target: POINT, rects: [RECT; 2] }

/// None means cache construction was unavailable: use the original full scan.
/// Some(None) is a verified empty point in the complete geometry index.
#[allow(clippy::option_option)] // Unavailable, verified empty, or a matching native region.
pub(super) fn hit(view: HWND, x: i32, y: i32) -> Option<Option<(i32, u32)>> {
    // The separate native-pane experiment changes its mapping every move frame.
    // A full index would be cold every time there; keep its short native scan.
    let hybrid = STATE.with(|s| s.borrow().as_ref().is_some_and(|s| s.panes.is_empty()
        && (s.identities.active || s.members.values().any(|pane| *pane == super::super::protocol::HIDDEN_ITEM as usize - 1))));
    if !hybrid { return None; }
    let cached = STATE.with(|s| s.borrow().as_ref().and_then(|s| s.cache.hits.clone()));
    let index = if let Some(index) = cached { index } else {
        let (revision, targets) = STATE.with(|s| s.borrow().as_ref().map(|s|
            (s.cache.revision, s.targets.iter().map(|(&i, &p)| (i, p)).collect::<Vec<_>>())))?;
        let mut index = Spatial::new();
        for (item, target) in targets {
            let mut rects = [RECT::default(); 2];
            for (rect, kind) in rects.iter_mut().zip([LVIR_ICON, LVIR_LABEL]) {
                rect.left = kind as i32;
                if unsafe { SendMessageW(view, LVM_GETITEMRECT, usize::try_from(item).ok()?, std::ptr::from_mut(rect) as isize) } == 0 { return None; }
            }
            index.add(Hit { item, target, rects }, &rects);
        }
        let index = Rc::new(index);
        let valid = STATE.with(|s| {
            let mut s = s.borrow_mut();
            let Some(s) = s.as_mut().filter(|s| s.cache.revision == revision) else { return false; };
            s.cache.hits = Some(index.clone());
            true
        });
        if !valid { return None; }
        index
    };
    let mut candidates = index.candidates(&RECT { left: x, top: y, right: x.saturating_add(1), bottom: y.saturating_add(1) });
    candidates.sort_by_key(|&i| {
        let entry = &index.entries[i];
        let dx = i64::from(x) - i64::from(entry.target.x);
        let dy = i64::from(y) - i64::from(entry.target.y);
        (dx * dx + dy * dy, entry.item)
    });
    for i in candidates {
        let entry = &index.entries[i];
        if !super::visible_hit(entry.item, x, y) { continue; }
        for (rect, flag) in entry.rects.iter().zip([2, 4]) {
            if contains(rect, x, y) { return Some(Some((entry.item, flag))); }
        }
    }
    Some(None)
}

pub(super) fn bounds(view: HWND, item: i32) -> Option<(RECT, POINT)> {
    let (revision, cached) = STATE.with(|s| s.borrow().as_ref().map(|s| (s.cache.revision, s.cache.bounds.get(&item).copied())))?;
    if cached.is_some() { return cached; }
    let bounds = super::baseline_bounds(view, item)?;
    STATE.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut().filter(|s| s.cache.revision == revision) { s.cache.bounds.insert(item, bounds); }
    });
    Some(bounds)
}

#[derive(Clone, Copy)]
pub(super) struct Damage { pub native: RECT, pub mapped: RECT }
pub(super) fn damage(view: HWND, region: &RECT) -> Option<Vec<Damage>> {
    let cached = STATE.with(|s| s.borrow().as_ref().and_then(|s| s.cache.damage.clone()));
    let index = if let Some(index) = cached { index } else {
        let (revision, targets) = STATE.with(|s| s.borrow().as_ref().map(|s|
            (s.cache.revision, s.targets.iter().map(|(&i, &p)| (i, p)).collect::<Vec<_>>())))?;
        let mut index = Spatial::new();
        for (item, target) in targets {
            let (native, baseline) = bounds(view, item)?;
            let mapped = super::offset_bounds(native, baseline, Some(&target));
            index.add(Damage { native, mapped }, &[native, mapped]);
        }
        let index = Rc::new(index);
        let valid = STATE.with(|s| {
            let mut s = s.borrow_mut();
            let Some(s) = s.as_mut().filter(|s| s.cache.revision == revision) else { return false; };
            s.cache.damage = Some(index.clone());
            true
        });
        if !valid { return None; }
        index
    };
    Some(index.candidates(region).into_iter().map(|i| index.entries[i]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grid_candidates_cover_negative_overlapping_and_long_label_rectangles() {
        let rects = [RECT { left: -300, top: -20, right: 90, bottom: 400 },
            RECT { left: 70, top: 0, right: 200, bottom: 40 },
            RECT { left: -100_000, top: -100_000, right: 100_000, bottom: 100_000 }];
        let mut index = Spatial::new();
        for rect in rects { index.add(rect, &[rect]); }
        for x in (-400..400).step_by(11) { for y in (-100..500).step_by(13) {
            let candidates = index.candidates(&RECT { left: x, top: y, right: x + 1, bottom: y + 1 });
            for (i, rect) in rects.iter().enumerate() {
                if contains(rect, x, y) { assert!(candidates.contains(&i)); }
            }
        } }
    }
}
