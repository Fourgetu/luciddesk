//! Exact-image native geometry experiment. No work-area or item-position writes.
//! All transformed geometry comes from the control's original drawing functions.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::too_many_arguments
)]
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateRectRgn, DeleteObject, GetUpdateRgn, InvalidateRect, RectInRegion,
};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW,
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Controls::{
    LVHITTESTINFO, LVIR_BOUNDS, LVIR_ICON, LVIR_LABEL, LVM_DELETEALLITEMS, LVM_DELETEITEM,
    LVM_GETITEMCOUNT, LVM_GETITEMPOSITION, LVM_GETITEMRECT, LVM_HITTEST, LVM_INSERTITEMW,
    LVM_SETITEMCOUNT, LVS_ICON, LVS_OWNERDATA, LVS_TYPEMASK,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GCLP_WNDPROC, GWL_STYLE, GetClassLongPtrW, GetClassNameW, GetWindowLongW,
    GetWindowThreadProcessId, SendMessageW, WM_NCDESTROY, WM_PAINT,
};

#[path = "geometry_profile.rs"]
mod profile;
mod hidden;
mod drop_target;
mod insertion;

unsafe extern "system" {
    fn MH_Initialize() -> i32;
    fn MH_CreateHook(target: *mut c_void, detour: *mut c_void, original: *mut *mut c_void) -> i32;
    fn MH_EnableHook(target: *mut c_void) -> i32;
    fn MH_DisableHook(target: *mut c_void) -> i32;
    fn MH_RemoveHook(target: *mut c_void) -> i32;
}

type GetRects = unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    i32,
    i32,
    u32,
    *mut RECT,
    *mut RECT,
    *mut c_void,
    i32,
);
type HitTest =
    unsafe extern "system" fn(*mut c_void, i32, i32, *mut u32, *mut i32, *mut i32) -> i32;
type GetPosition = unsafe extern "system" fn(*mut c_void, i32, i32, *mut POINT) -> i32;
type GetInsertRect = unsafe extern "system" fn(*mut c_void, *mut RECT) -> i32;
type HitInsertMark = unsafe extern "system" fn(*mut c_void, i32, i32, *mut windows_sys::Win32::UI::Controls::LVINSERTMARK) -> i32;
static ORIGINAL_RECTS: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_HIT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_POSITION: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_INSERT_RECT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_INSERT_HIT: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: OnceLock<Result<[usize; 5], String>> = OnceLock::new();
static ACTIVE: Mutex<bool> = Mutex::new(false);
const SUBCLASS: usize = 0x4c50_4745;

struct State {
    view: HWND,
    objects: [usize; 4],
    targets: BTreeMap<i32, POINT>,
    pending: Option<BTreeMap<i32, POINT>>,
    reads: u64,
    panes: Vec<crate::protocol::PaneAppearance>,
    members: BTreeMap<i32, usize>,
    surface: crate::pane_surface::Surface,
    clip: Option<(windows_sys::Win32::Graphics::Gdi::HDC, i32)>,
    identities: hidden::Identities,
}
thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static SCOPE: Cell<usize> = const { Cell::new(0) };
    static BYPASS: Cell<bool> = const { Cell::new(false) };
    static MOVE_REQUESTS: Cell<isize> = const { Cell::new(0) };
}

#[must_use]
pub fn move_requests() -> isize { MOVE_REQUESTS.get() }

fn bypass<T>(call: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            BYPASS.set(self.0);
        }
    }
    let _reset = Reset(BYPASS.replace(true));
    call()
}

fn matches(index: usize, object: *mut c_void) -> bool {
    if BYPASS.get() {
        return false;
    }
    STATE.with(|cell| {
        let Ok(mut state) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(state) = state.as_mut() else {
            return false;
        };
        if state.objects[index] == 0 && SCOPE.get() > 0 {
            state.objects[index] = object as usize;
        }
        state.objects[index] != 0 && state.objects[index] == object as usize
    })
}

fn target(item: i32) -> Option<(POINT, usize)> {
    STATE.with(|cell| {
        let state = cell.try_borrow().ok()?;
        let state = state.as_ref()?;
        Some((*state.targets.get(&item)?, state.objects[2]))
    })
}

unsafe extern "system" fn insert_rect(this: *mut c_void, rect: *mut RECT) -> i32 {
    let original: GetInsertRect = unsafe { std::mem::transmute(ORIGINAL_INSERT_RECT.load(Ordering::Acquire)) };
    if !matches(3, this) { return unsafe { original(this, rect) }; }
    // Shell's owner-data callback uses a different rectangle path from a plain
    // list control. Obtain native geometry explicitly, then translate exactly
    // once here so both indicator painting and rectangle queries agree.
    let ok = bypass(|| unsafe { original(this, rect) });
    if ok == 0 || rect.is_null() { return ok; }
    let view = STATE.with(|cell| cell.borrow().as_ref().map(|state| state.view));
    let Some(view) = view else { return ok; };
    let mut mark = windows_sys::Win32::UI::Controls::LVINSERTMARK {
        cbSize: std::mem::size_of::<windows_sys::Win32::UI::Controls::LVINSERTMARK>() as u32,
        iItem: -1,
        ..Default::default()
    };
    unsafe { SendMessageW(view, windows_sys::Win32::UI::Controls::LVM_GETINSERTMARK, 0, (&raw mut mark) as isize); }
    let marker_target = target(mark.iItem).map(|(destination, object)| {
        let destination = insertion::hidden_marker_anchor(mark.iItem,
            mark.dwFlags & windows_sys::Win32::UI::Controls::LVIM_AFTER != 0).unwrap_or(destination);
        (destination, object)
    });
    if let Some((destination, object)) = marker_target.filter(|(_, object)| *object != 0) {
        let get_position: GetPosition = unsafe { std::mem::transmute(ORIGINAL_POSITION.load(Ordering::Acquire)) };
        let mut baseline = POINT::default();
        if bypass(|| unsafe { get_position(object as _, mark.iItem, -1, &raw mut baseline) }) != 0 {
            let dx = destination.x.saturating_sub(baseline.x);
            let dy = destination.y.saturating_sub(baseline.y);
            unsafe {
                (*rect).left = (*rect).left.saturating_add(dx);
                (*rect).right = (*rect).right.saturating_add(dx);
                (*rect).top = (*rect).top.saturating_add(dy);
                (*rect).bottom = (*rect).bottom.saturating_add(dy);
            }
        }
    }
    ok
}

unsafe extern "system" fn insert_hit(this: *mut c_void, x: i32, y: i32, mark: *mut windows_sys::Win32::UI::Controls::LVINSERTMARK) -> i32 {
    let original: HitInsertMark = unsafe { std::mem::transmute(ORIGINAL_INSERT_HIT.load(Ordering::Acquire)) };
    if !matches(2, this) { return unsafe { original(this, x, y, mark) }; }
    // Native insertion hit-testing suppresses the source and its adjacent no-op
    // gaps BEFORE returning an index. Remapping the output is too late: it can
    // already be -1 for an unrelated visible gap. Resolve the input cell first,
    // then let native code apply its source/no-op rules in one coordinate space.
    if let Some(point) = insertion::native_point(POINT { x, y }) {
        let result = bypass(|| unsafe { original(this, point.x, point.y, mark) });
        if result != 0 && !mark.is_null() {
            insertion::normalize_mark(unsafe { &mut *mark });
        }
        return result;
    }
    // Retain the existing mapping for the separate native-pane experiment.
    let ok = bypass(|| unsafe { original(this, x, y, mark) });
    if ok == 0 || mark.is_null() || unsafe { (*mark).iItem } < 0 { return ok; }
    let get_position: GetPosition = unsafe { std::mem::transmute(ORIGINAL_POSITION.load(Ordering::Acquire)) };
    let mut slot = POINT::default();
    if bypass(|| unsafe { get_position(this, (*mark).iItem, -1, &raw mut slot) }) == 0 { return ok; }
    let mapped = STATE.with(|cell| {
        let state = cell.borrow();
        let state = state.as_ref()?;
        state.targets.iter().find(|(item, p)| !hidden::is_hidden(**item) && p.x == slot.x && p.y == slot.y).map(|(item, _)| *item)
    });
    if let Some(item) = mapped {
        unsafe { (*mark).iItem = item; }
    }
    ok
}

unsafe extern "system" fn rects(
    this: *mut c_void,
    draw: *mut c_void,
    item: i32,
    group: i32,
    flags: u32,
    icon: *mut RECT,
    label: *mut RECT,
    list_item: *mut c_void,
    mode: i32,
) {
    let original: GetRects = unsafe { std::mem::transmute(ORIGINAL_RECTS.load(Ordering::Acquire)) };
    let scoped = matches(0, this);
    // Shell's owner-data drawing callback can query item positions recursively.
    // Keep the entire native rectangle calculation in native coordinates, then
    // apply the presentation delta once. Otherwise the callback can read a
    // translated position and this outer call translates it a second time.
    bypass(|| unsafe { original(this, draw, item, group, flags, icon, label, list_item, mode) });
    if scoped && let Some((destination, object)) = target(item).filter(|(_, object)| *object != 0) {
        let original_position: GetPosition =
            unsafe { std::mem::transmute(ORIGINAL_POSITION.load(Ordering::Acquire)) };
        let mut baseline = POINT::default();
        let ok =
            bypass(|| unsafe { original_position(object as _, item, group, &raw mut baseline) });
        if ok != 0 {
            let dx = destination.x.saturating_sub(baseline.x);
            let dy = destination.y.saturating_sub(baseline.y);
            // Some callers alias output pointers. Each rectangle must be offset once.
            let outputs = [icon, label];
            for (index, ptr) in outputs.iter().enumerate() {
                if !ptr.is_null() && !outputs[..index].contains(ptr) {
                    let rect = unsafe { &mut **ptr };
                    rect.left = rect.left.saturating_add(dx);
                    rect.right = rect.right.saturating_add(dx);
                    rect.top = rect.top.saturating_add(dy);
                    rect.bottom = rect.bottom.saturating_add(dy);
                }
            }
            STATE.with(|cell| {
                if let Ok(mut state) = cell.try_borrow_mut()
                    && let Some(state) = state.as_mut()
                {
                    state.reads += 1;
                }
            });
        }
    }
}

unsafe extern "system" fn position(
    this: *mut c_void,
    item: i32,
    group: i32,
    output: *mut POINT,
) -> i32 {
    let original: GetPosition =
        unsafe { std::mem::transmute(ORIGINAL_POSITION.load(Ordering::Acquire)) };
    let scoped = matches(2, this);
    // The original auto-arrange implementation itself calls GetRects; suppress translation
    // until it returns, otherwise relative offsets would be applied twice.
    let result = bypass(|| unsafe { original(this, item, group, output) });
    if scoped
        && result != 0
        && !output.is_null()
        && let Some((point, _)) = target(item)
    {
        unsafe {
            *output = point;
        }
    }
    result
}

fn contains(rect: &RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

fn visible_hit(item: i32, x: i32, y: i32) -> bool {
    if hidden::is_hidden(item) { return false; }
    STATE.with(|s| {
        let s = s.borrow();
        let Some(s) = s.as_ref() else {
            return true;
        };
        let pane = s
            .panes
            .iter()
            .rposition(|p| crate::pane_surface::inside(p, x, y));
        match s.members.get(&item) {
            Some(&member) => {
                pane == Some(member)
                    && y >= s.panes[member].bounds.top + crate::protocol::PANE_HEADER
            }
            None => pane.is_none(),
        }
    })
}

fn restore_item_clip() {
    let clip = STATE.with(|s| s.borrow_mut().as_mut().and_then(|s| s.clip.take()));
    if let Some((dc, saved)) = clip {
        unsafe {
            windows_sys::Win32::Graphics::Gdi::RestoreDC(dc, saved);
        }
    }
}

unsafe extern "system" fn parent_subclass(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    _: usize,
) -> isize {
    use windows_sys::Win32::Graphics::Gdi::{SaveDC, ExtSelectClipRgn, RGN_AND, IntersectClipRect, RGN_DIFF, DeleteObject};
    use windows_sys::Win32::UI::Controls::{NMHDR, NM_CUSTOMDRAW, NMLVCUSTOMDRAW, CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDRF_NOTIFYITEMDRAW, CDRF_NOTIFYPOSTPAINT, CDRF_SKIPDEFAULT, CDDS_ITEMPOSTPAINT, CDDS_POSTPAINT};
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_NOTIFY;
    if msg != WM_NOTIFY || lp == 0 {
        return unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
    }
    let header = unsafe { &*(lp as *const NMHDR) };
    if let Some(result) = unsafe { hidden::notification(header, lp) } { return result; }
    let ours = STATE.with(|s| {
        s.borrow()
            .as_ref()
                .is_some_and(|s| s.view == header.hwndFrom && (!s.panes.is_empty() || !s.members.is_empty() || s.identities.active))
    });
    if !ours || header.code != NM_CUSTOMDRAW {
        return unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
    }
    let draw = unsafe { &*(lp as *const NMLVCUSTOMDRAW) };
    let stage = draw.nmcd.dwDrawStage;
    if stage == CDDS_ITEMPREPAINT && hidden::is_hidden(draw.nmcd.dwItemSpec as i32) {
        restore_item_clip();
        return CDRF_SKIPDEFAULT as isize;
    }
    if stage == CDDS_ITEMPREPAINT {
        restore_item_clip();
        let dc = draw.nmcd.hdc;
        let saved = unsafe { SaveDC(dc) };
        if saved != 0 {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                let Some(s) = s.as_mut() else {
                    return;
                };
                let member = s.members.get(&(draw.nmcd.dwItemSpec as i32)).copied();
                for (index, pane) in s.panes.iter().enumerate() {
                    let region = crate::pane_surface::pane_region(pane);
                    if region.is_null() {
                        continue;
                    }
                    unsafe {
                        if member == Some(index) {
                            ExtSelectClipRgn(dc, region, RGN_AND);
                            IntersectClipRect(
                                dc,
                                pane.bounds.left,
                                pane.bounds.top + crate::protocol::PANE_HEADER,
                                pane.bounds.right,
                                pane.bounds.bottom,
                            );
                        } else {
                            ExtSelectClipRgn(dc, region, RGN_DIFF);
                        }
                        DeleteObject(region);
                    }
                }
                s.clip = Some((dc, saved));
            });
        }
    }
    let original = unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
    if stage == CDDS_PREPAINT {
        STATE.with(|s| {
            let s = s.borrow();
            if let Some(s) = s.as_ref() {
                s.surface.paint(draw.nmcd.hdc, &s.panes);
            }
        });
        original | (CDRF_NOTIFYITEMDRAW | CDRF_NOTIFYPOSTPAINT) as isize
    } else if stage == CDDS_ITEMPREPAINT {
        if original & CDRF_SKIPDEFAULT as isize != 0 {
            restore_item_clip();
        }
        original | CDRF_NOTIFYPOSTPAINT as isize
    } else {
        if stage == CDDS_ITEMPOSTPAINT || stage == CDDS_POSTPAINT {
            restore_item_clip();
        }
        original
    }
}

fn same_position(a: Option<&POINT>, b: Option<&POINT>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.x == b.x && a.y == b.y,
        (None, None) => true,
        _ => false,
    }
}

/// Native bounds include the selected label. Keep a small margin for desktop shadows.
fn baseline_bounds(view: HWND, item: i32) -> Option<(RECT, POINT)> {
    let index = usize::try_from(item).ok()?;
    let mut bounds = RECT {
        left: LVIR_BOUNDS as i32,
        ..RECT::default()
    };
    let mut point = POINT::default();
    let ok = bypass(|| unsafe {
        SendMessageW(view, LVM_GETITEMRECT, index, (&raw mut bounds) as isize) != 0
            && SendMessageW(view, LVM_GETITEMPOSITION, index, (&raw mut point) as isize) != 0
    });
    if !ok {
        return None;
    }
    bounds.left -= 8;
    bounds.top -= 8;
    bounds.right += 8;
    bounds.bottom += 8;
    Some((bounds, point))
}

fn offset_bounds(mut bounds: RECT, baseline: POINT, target: Option<&POINT>) -> RECT {
    if let Some(target) = target {
        let dx = target.x - baseline.x;
        let dy = target.y - baseline.y;
        bounds.left += dx;
        bounds.right += dx;
        bounds.top += dy;
        bounds.bottom += dy;
    }
    bounds
}

fn invalidate_mapping_delta(view: HWND, old: &BTreeMap<i32, POINT>, new: &BTreeMap<i32, POINT>) {
    let mut indices: Vec<_> = old.keys().chain(new.keys()).copied().collect();
    indices.sort_unstable();
    indices.dedup();
    for item in indices {
        if same_position(old.get(&item), new.get(&item)) {
            continue;
        }
        let Some((bounds, baseline)) = baseline_bounds(view, item) else {
            continue;
        };
        let before = offset_bounds(bounds, baseline, old.get(&item));
        let after = offset_bounds(bounds, baseline, new.get(&item));
        // Include the original cell so the native virtual-grid culler visits this item.
        // A complex update region keeps unrelated desktop pixels outside the paint clip.
        unsafe {
            InvalidateRect(view, &raw const bounds, 0);
            InvalidateRect(view, &raw const before, 0);
            InvalidateRect(view, &raw const after, 0);
        }
    }
}

fn expand_native_damage(view: HWND) {
    let targets = STATE.with(|state| state.borrow().as_ref().map(|s| s.targets.clone()));
    let Some(targets) = targets.filter(|t| !t.is_empty()) else {
        return;
    };
    unsafe {
        let damage = CreateRectRgn(0, 0, 0, 0);
        if damage.is_null() {
            return;
        }
        if GetUpdateRgn(view, damage, 0) > 1 {
            for (item, target) in targets {
                let Some((bounds, baseline)) = baseline_bounds(view, item) else {
                    continue;
                };
                let mapped = offset_bounds(bounds, baseline, Some(&target));
                if RectInRegion(damage, &raw const bounds) != 0
                    || RectInRegion(damage, &raw const mapped) != 0
                {
                    InvalidateRect(view, &raw const bounds, 0);
                    InvalidateRect(view, &raw const mapped, 0);
                }
            }
        }
        DeleteObject(damage);
    }
}

unsafe extern "system" fn hit(
    this: *mut c_void,
    x: i32,
    y: i32,
    flags: *mut u32,
    subitem: *mut i32,
    group: *mut i32,
) -> i32 {
    let original: HitTest = unsafe { std::mem::transmute(ORIGINAL_HIT.load(Ordering::Acquire)) };
    if !matches(1, this) {
        return unsafe { original(this, x, y, flags, subitem, group) };
    }
    let snapshot = STATE.with(|cell| {
        cell.try_borrow().ok().and_then(|s| {
            s.as_ref()
                .map(|s| (s.view, s.targets.clone()))
        })
    });
    if let Some((view, targets)) = snapshot {
        let mut candidates: Vec<_> = targets.iter().collect();
        candidates.sort_by_key(|(_, p)| {
            let dx = i64::from(x) - i64::from(p.x);
            let dy = i64::from(y) - i64::from(p.y);
            dx * dx + dy * dy
        });
        for (&item, _) in candidates {
            if !visible_hit(item, x, y) { continue; }
            for kind in [LVIR_ICON, LVIR_LABEL] {
                let mut rect = RECT {
                    left: kind as i32,
                    ..RECT::default()
                };
                let ok = unsafe {
                    SendMessageW(
                        view,
                        LVM_GETITEMRECT,
                        usize::try_from(item).unwrap_or(usize::MAX),
                        (&raw mut rect) as isize,
                    )
                };
                if ok != 0 && contains(&rect, x, y) {
                    // The displayed native icon/label rectangle already identifies
                    // the item. Asking the underlying virtual grid again can resolve
                    // a collected item's old slot and reject this visible icon.
                    if !flags.is_null() { unsafe { *flags = if kind == LVIR_ICON { 2 } else { 4 }; } }
                    if !subitem.is_null() { unsafe { *subitem = 0; } }
                    if !group.is_null() { unsafe { *group = -1; } }
                    return item;
                }
            }
        }
        let result = bypass(|| unsafe { original(this, x, y, flags, subitem, group) });
        if targets.contains_key(&result) || !visible_hit(result, x, y) {
            if !flags.is_null() {
                unsafe {
                    *flags = 1;
                }
            }
            if !subitem.is_null() {
                unsafe {
                    *subitem = -1;
                }
            }
            if !group.is_null() {
                unsafe {
                    *group = -1;
                }
            }
            return -1;
        }
        return result;
    }
    unsafe { original(this, x, y, flags, subitem, group) }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    message: u32,
    wp: usize,
    lp: isize,
    _: usize,
    _: usize,
) -> isize {
    struct Scope;
    impl Drop for Scope {
        fn drop(&mut self) {
            SCOPE.set(SCOPE.get().saturating_sub(1));
        }
    }
    SCOPE.set(SCOPE.get() + 1);
    let _scope = Scope;
    if message == windows_sys::Win32::UI::Controls::LVM_SETITEMPOSITION
        || message == windows_sys::Win32::UI::Controls::LVM_SETITEMPOSITION32 {
        MOVE_REQUESTS.set(MOVE_REQUESTS.get().saturating_add(1));
    }
    if let Some(result) = unsafe { hidden::message(hwnd, message, wp, lp) } { return result; }
    if message == WM_NCDESTROY {
        STATE.with(|state| {
            state.borrow_mut().take();
        });
        unsafe {
            RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS);
        }
    } else if [
        LVM_SETITEMCOUNT,
        LVM_DELETEALLITEMS,
        LVM_DELETEITEM,
        LVM_INSERTITEMW,
        windows_sys::Win32::UI::Controls::LVM_SORTITEMS,
        windows_sys::Win32::UI::Controls::LVM_SORTITEMSEX,
    ]
    .contains(&message)
    {
        // View indices can change. Discard offsets before the original control sees the change.
        hidden::invalidate_indices();
        STATE.with(|state| {
            if let Some(state) = state.borrow_mut().as_mut() {
                state.targets.clear();
                state.pending = None;
                state.members.clear();
            }
        });
    } else if message == WM_PAINT {
        hidden::refresh_identities(hwnd);
        expand_native_damage(hwnd);
    }
    unsafe { DefSubclassProc(hwnd, message, wp, lp) }
}

/// A same-UI-thread native geometry session. Icons and native automatic-arrangement flags remain
/// owned by the existing control. Only the reviewed exact comctl32 image is accepted.
pub struct GeometrySession {
    drop_target: Option<drop_target::Registration>,
    view: HWND,
    parent: HWND,
    _thread: PhantomData<Rc<()>>,
}

impl GeometrySession {
    /// Query the same native insertion hit-test used by Shell's list-control interface.
    #[must_use]
    pub fn insertion_target(&self, point: POINT) -> Option<(i32, bool)> {
        use windows_sys::Win32::UI::Controls::{LVINSERTMARK, LVIM_AFTER};
        let object = STATE.with(|s| s.borrow().as_ref().map_or(0, |s| s.objects[2]));
        if object == 0 { return None; }
        let mut mark = LVINSERTMARK { cbSize: std::mem::size_of::<LVINSERTMARK>() as u32, iItem: -1, ..Default::default() };
        let ok = unsafe { insert_hit(object as _, point.x, point.y, &raw mut mark) };
        (ok != 0 && mark.iItem >= 0).then_some((mark.iItem, mark.dwFlags & LVIM_AFTER != 0))
    }

    #[must_use]
    pub fn has_drop_proxy(&self) -> bool { self.drop_target.is_some() }
    /// Cache validated labels only to bridge the short interval during a Shell reorder.
    /// Ambiguous labels are never used to resolve a new hidden index.
    pub fn identities(&self, items: &[crate::protocol::ItemPosition], owner: isize) {
        hidden::set_identities(items, owner);
        hidden::clear_selection(self.view);
    }
    /// Temporarily allow Shell to select hidden items for a user-requested menu.
    pub fn menu_selection(&self, allow: bool) {
        hidden::MENU.set(allow);
        if !allow { hidden::clear_selection(self.view); }
    }
    /// # Safety
    /// `view` must be a live `ListView` on this process and calling UI thread. Keep its message
    /// loop running and drop the session on this same thread before unloading application code.
    /// # Errors
    /// Returns an error on an unsupported image/style, competing session or failed detour.
    pub unsafe fn attach(view: isize) -> Result<Self, String> {
        let view = view as HWND;
        let surface = crate::pane_surface::Surface::new()?;
        let parent = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(view) };
        if parent.is_null() {
            return Err("原生视图缺少父窗口".into());
        }
        if unsafe { GetWindowThreadProcessId(view, null_mut()) } != unsafe { GetCurrentThreadId() }
        {
            return Err("Geometry Hook 必须在目标图标视图线程中初始化".into());
        }
        let mut class = [0_u16; 64];
        let n = unsafe { GetClassNameW(view, class.as_mut_ptr(), 64) };
        if n <= 0
            || String::from_utf16_lossy(&class[..usize::try_from(n).unwrap_or(0)])
                != "SysListView32"
        {
            return Err("Geometry Hook 目标不是 ListView".into());
        }
        let style = unsafe { GetWindowLongW(view, GWL_STYLE) }.cast_unsigned();
        if style & LVS_OWNERDATA == 0 || style & LVS_TYPEMASK != LVS_ICON {
            return Err("Geometry Hook 只接受虚拟大图标视图".into());
        }
        let mut active = ACTIVE.lock().map_err(|_| "Hook 状态锁异常")?;
        if *active {
            return Err("进程内已有 Geometry Hook 会话".into());
        }
        let targets = INSTALLED
            .get_or_init(|| unsafe { install(view) })
            .as_ref()
            .map_err(Clone::clone)?;
        for &address in targets {
            let status = unsafe { MH_EnableHook(address as _) };
            if status != 0 && status != 5 {
                for &address in targets {
                    unsafe {
                        MH_DisableHook(address as _);
                    }
                }
                return Err(format!("启用原生几何 Hook 失败：{status}"));
            }
        }
        STATE.with(|cell| {
            *cell.borrow_mut() = Some(State {
                view,
                objects: [0; 4],
                targets: BTreeMap::new(),
                pending: None,
                reads: 0,
                panes: Vec::new(),
                members: BTreeMap::new(),
                surface,
                clip: None,
                identities: hidden::Identities::default(),
            });
        });
        if unsafe { SetWindowSubclass(view, Some(subclass), SUBCLASS, 0) } == 0 {
            STATE.with(|cell| {
                cell.borrow_mut().take();
            });
            for &address in targets {
                unsafe {
                    MH_DisableHook(address as _);
                }
            }
            return Err("安装原生几何消息作用域失败".into());
        }
        *active = true;
        drop(active);
        let mut session = Self {
            drop_target: None,
            view,
            parent,
            _thread: PhantomData,
        };
        if unsafe { SetWindowSubclass(parent, Some(parent_subclass), SUBCLASS + 1, 0) } == 0 {
            return Err("无法安装原生分组绘制回调".into());
        }
        if unsafe { SendMessageW(view, LVM_GETITEMCOUNT, 0, 0) } > 0 {
            let mut rect = RECT::default();
            let mut point = POINT::default();
            unsafe {
                SendMessageW(view, LVM_GETITEMRECT, 0, (&raw mut rect) as isize);
                SendMessageW(view, LVM_GETITEMPOSITION, 0, (&raw mut point) as isize);
                SendMessageW(view, windows_sys::Win32::UI::Controls::LVM_GETINSERTMARKRECT, 0, (&raw mut rect) as isize);
                let mut hit = LVHITTESTINFO {
                    pt: POINT {
                        x: point.x + 2,
                        y: point.y + 2,
                    },
                    ..LVHITTESTINFO::default()
                };
                SendMessageW(view, LVM_HITTEST, 0, (&raw mut hit) as isize);
            }
            let objects = STATE.with(|s| s.borrow().as_ref().map_or([0; 4], |s| s.objects));
            if objects.contains(&0) || objects[0] != objects[2] {
                return Err(format!("原生几何函数绑定不完整：{objects:x?}"));
            }
        }
        session.drop_target = unsafe { drop_target::Registration::attach(view) }?;
        Ok(session)
    }

    /// Atomically replace desired icon positions in the view's client coordinates.
    /// # Errors
    /// Rejects stale/out-of-range indices or a destroyed target view.
    pub fn set_positions(&self, positions: &[(i32, POINT)]) -> Result<(), String> {
        let count = unsafe { SendMessageW(self.view, LVM_GETITEMCOUNT, 0, 0) };
        if positions.len() > 8192
            || positions.iter().any(|(i, p)| {
                *i < 0
                    || i64::from(*i) >= count as i64
                    || p.x.abs_diff(0) > 100_000
                    || p.y.abs_diff(0) > 100_000
            })
        {
            return Err("无效的原生图标目标位置".into());
        }
        let next: BTreeMap<_, _> = positions.iter().copied().collect();
        let previous = STATE.with(|cell| {
            let mut state = cell.borrow_mut();
            let state = state.as_mut().ok_or("原生图标视图已退出")?;
            Ok::<_, String>(std::mem::replace(&mut state.targets, next.clone()))
        })?;
        invalidate_mapping_delta(self.view, &previous, &next);
        Ok(())
    }

    /// Update one mapped icon while retaining the other mapped positions.
    /// # Errors
    /// Rejects a stale index or destroyed target.
    pub fn set_position(&self, item: i32, point: POINT) -> Result<(), String> {
        let count = unsafe { SendMessageW(self.view, LVM_GETITEMCOUNT, 0, 0) };
        if item < 0
            || item as isize >= count
            || point.x.abs_diff(0) > 100_000
            || point.y.abs_diff(0) > 100_000
        {
            return Err("无效的原生图标位置".into());
        }
        let staged = STATE.with(|cell| {
            let mut state = cell.borrow_mut();
            if let Some(pending) = state.as_mut().and_then(|s| s.pending.as_mut()) {
                pending.insert(item, point);
                true
            } else {
                false
            }
        });
        if staged {
            return Ok(());
        }
        let mut positions = STATE.with(|cell| {
            cell.borrow()
                .as_ref()
                .map_or_else(BTreeMap::new, |s| s.targets.clone())
        });
        positions.insert(item, point);
        self.set_positions(&positions.into_iter().collect::<Vec<_>>())
    }

    /// Begin a frame update; the current mapping remains visible until commit.
    pub fn begin_positions(&self) {
        STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.pending = Some(BTreeMap::new());
            }
        });
    }

    /// Read the native arrangement underneath the geometry mapping.
    /// # Errors
    /// Fails when the native view rejects the index.
    pub fn original_position(&self, item: i32) -> Result<POINT, String> {
        if item < 0 {
            return Err("无效图标索引".into());
        }
        let mut point = POINT::default();
        let ok = bypass(|| unsafe {
            SendMessageW(
                self.view,
                LVM_GETITEMPOSITION,
                usize::try_from(item).unwrap_or(usize::MAX),
                (&raw mut point) as isize,
            )
        });
        if ok == 0 {
            Err("无法读取原生图标位置".into())
        } else {
            Ok(point)
        }
    }

    /// Publish a staged frame in one repaint.
    /// # Errors
    /// Returns an error if no update is staged or the view has been destroyed.
    pub fn commit_positions(&self) -> Result<(), String> {
        let (previous, next) = STATE.with(|cell| {
            let mut state = cell.borrow_mut();
            let state = state.as_mut().ok_or("图标视图已退出")?;
            let next = state.pending.take().ok_or("没有待提交的布局")?;
            let previous = std::mem::replace(&mut state.targets, next.clone());
            Ok::<_, String>((previous, next))
        })?;
        invalidate_mapping_delta(self.view, &previous, &next);
        Ok(())
    }

    /// Publish backgrounds, membership and icon coordinates as a single native scene.
    /// # Errors
    /// Fails if no transaction is staged or the view has exited.
    pub fn commit_scene(
        &self,
        panes: &[crate::protocol::PaneAppearance],
        members: BTreeMap<i32, usize>,
    ) -> Result<(), String> {
        let mut membership_changed = false;
        let previous = STATE
            .with(|s| {
                let mut s = s.borrow_mut();
                let s = s.as_mut().ok_or("原生图标视图已退出")?;
                if s.pending.is_none() {
                    return Err("没有待提交的布局");
                }
                membership_changed = s.members != members;
                s.members = members;
                Ok(std::mem::replace(&mut s.panes, panes.to_vec()))
            })
            .map_err(str::to_string)?;
        self.commit_positions()?;
        hidden::clear_selection(self.view);
        if membership_changed { unsafe { InvalidateRect(self.view, std::ptr::null(), 0); } }
        for (index,pane) in previous.iter().enumerate().filter(|(i,p)|panes.get(*i)!=Some(*p))
            .chain(panes.iter().enumerate().filter(|(i,p)|previous.get(*i)!=Some(*p))) {
            let _ = index;
            let b = pane.bounds;
            let rect = RECT {
                left: b.left,
                top: b.top,
                right: b.right,
                bottom: b.bottom,
            };
            unsafe {
                InvalidateRect(self.view, &raw const rect, 0);
            }
        }
        Ok(())
    }

    /// Installs a bounded, already decoded wallpaper texture. No image codecs run in Explorer.
    /// # Errors
    /// Rejects invalid dimensions or allocation failure.
    pub fn set_texture(
        &self,
        header: crate::protocol::TextureHeader,
        pixels: &[u8],
    ) -> Result<(), String> {
        STATE.with(|s| {
            s.borrow_mut()
                .as_mut()
                .ok_or("原生视图已退出")?
                .surface
                .texture(header, pixels)
        })
    }

    #[must_use]
    pub fn translated_rectangles(&self) -> u64 {
        STATE.with(|state| state.borrow().as_ref().map_or(0, |s| s.reads))
    }
}

impl Drop for GeometrySession {
    fn drop(&mut self) {
        self.drop_target.take();
        restore_item_clip();
        STATE.with(|cell| {
            cell.borrow_mut().take();
        });
        unsafe {
            RemoveWindowSubclass(self.view, Some(subclass), SUBCLASS);
            RemoveWindowSubclass(self.parent, Some(parent_subclass), SUBCLASS + 1);
        }
        if let Some(Ok(targets)) = INSTALLED.get() {
            for &target in targets {
                unsafe {
                    MH_DisableHook(target as _);
                }
            }
        }
        // Keep disabled trampolines allocated: another thread may still be finishing an
        // original call. The pinned code cannot be unloaded underneath that call.
        if let Ok(mut active) = ACTIVE.lock() {
            *active = false;
        }
        unsafe {
            InvalidateRect(self.view, null(), 0);
        }
    }
}

unsafe fn install(view: HWND) -> Result<[usize; 5], String> {
    let procedure = unsafe { GetClassLongPtrW(view, GCLP_WNDPROC) };
    let mut module = null_mut();
    if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            procedure as _,
            &raw mut module,
        )
    } == 0
    {
        return Err("无法定位图标控件模块".into());
    }
    let mut path = [0_u16; 4096];
    let length = unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), 4096) };
    if length == 0 || length >= 4096 {
        return Err("无法读取图标控件模块路径".into());
    }
    let bytes = std::fs::read(String::from_utf16_lossy(&path[..length as usize]))
        .map_err(|e| e.to_string())?;
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
    });
    if hash != profile::IMAGE_HASH || bytes.len() != profile::IMAGE_SIZE {
        return Err(format!(
            "当前控件版本没有经过验证的 Hook 配置（{hash:016x}）"
        ));
    }
    let targets = profile::TARGETS.map(|(rva, _)| module as usize + rva);
    for (&address, (_, expected)) in targets.iter().zip(profile::TARGETS) {
        if unsafe { std::slice::from_raw_parts(address as *const u8, expected.len()) } != expected {
            return Err("图标控件函数已被其他组件修改，未安装 Hook".into());
        }
    }
    let mut own_module = null_mut();
    if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            (rects as *const ()).cast(),
            &raw mut own_module,
        )
    } == 0
    {
        return Err("无法保护 Hook 回调生命周期".into());
    }
    let status = unsafe { MH_Initialize() };
    if status != 0 && status != 1 {
        return Err(format!("MinHook 初始化失败：{status}"));
    }
    let callbacks = [
        rects as *const () as *mut c_void,
        hit as *const () as _,
        position as *const () as _,
        insert_rect as *const () as _,
        insert_hit as *const () as _,
    ];
    let originals = [&ORIGINAL_RECTS, &ORIGINAL_HIT, &ORIGINAL_POSITION, &ORIGINAL_INSERT_RECT, &ORIGINAL_INSERT_HIT];
    for index in 0..targets.len() {
        let mut original = null_mut();
        let status =
            unsafe { MH_CreateHook(targets[index] as _, callbacks[index], &raw mut original) };
        if status != 0 {
            for &address in &targets[..index] {
                unsafe {
                    MH_RemoveHook(address as _);
                }
            }
            return Err(format!("创建原生几何 Hook 失败：{status}"));
        }
        originals[index].store(original as usize, Ordering::Release);
    }
    Ok(targets)
}
