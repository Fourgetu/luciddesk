//! Input isolation for presentation-only hidden items. No Shell items are deleted.
#![allow(
    clippy::wildcard_imports,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
use super::STATE;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::{Controls::*, Input::KeyboardAndMouse::*, Shell::DefSubclassProc, WindowsAndMessaging::*},
};
thread_local! {
    pub(super) static MENU: Cell<bool> = const { Cell::new(false) };
    pub(super) static RENAME_REQUESTED: Cell<bool> = const { Cell::new(false) };
    static CLEANING: Cell<bool> = const { Cell::new(false) };
}

#[derive(Default)]
pub(super) struct Identities {
    expected: BTreeMap<i32, u64>,
    unique_hidden: BTreeSet<u64>,
    current_hidden: BTreeSet<i32>,
    owner: isize,
    notified: bool,
    pub(super) active: bool,
}
pub(super) fn invalidate_indices() {
    STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().as_mut()
            && state.identities.active
        {
            state.identities.expected.clear();
            state.identities.current_hidden.clear();
            state.identities.notified = false;
            state.cache.layout_changed();
        }
    });
}
pub(super) fn set_identities(items: &[crate::protocol::ItemPosition], owner: isize) {
    let mut counts = BTreeMap::new();
    for item in items {
        *counts.entry(item.name_hash).or_insert(0usize) += 1;
    }
    let hidden: BTreeSet<_> = items
        .iter()
        .filter(|i| i.reserved == crate::protocol::HIDDEN_ITEM)
        .map(|i| i.item)
        .collect();
    let unique_hidden = items
        .iter()
        .filter(|i| hidden.contains(&i.item) && counts.get(&i.name_hash) == Some(&1))
        .map(|i| i.name_hash)
        .collect();
    STATE.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.cache.layout_changed();
            s.identities = Identities {
                expected: items.iter().map(|i| (i.item, i.name_hash)).collect(),
                unique_hidden,
                current_hidden: hidden.clone(),
                active: !hidden.is_empty(),
                owner,
                notified: false,
            };
        }
    });
}

fn name_hash(view: HWND, index: i32) -> Option<u64> {
    let mut text = [0u16; 1024];
    let mut item = LVITEMW {
        mask: LVIF_TEXT,
        iItem: index,
        pszText: text.as_mut_ptr(),
        cchTextMax: 1024,
        ..Default::default()
    };
    if unsafe { SendMessageW(view, LVM_GETITEMW, 0, (&raw mut item) as isize) } == 0
        || item.pszText.is_null()
    {
        return None;
    }
    let mut len = 0;
    while len < 1023 && unsafe { *item.pszText.add(len) } != 0 {
        len += 1;
    }
    if len == 0 || len == 1023 {
        return None;
    }
    Some(crate::protocol::name_hash(
        unsafe { std::slice::from_raw_parts(item.pszText, len) }
            .iter()
            .copied(),
    ))
}

fn original_slots(
    view: HWND,
    count: isize,
) -> Option<Vec<(i32, windows_sys::Win32::Foundation::POINT)>> {
    (0..count)
        .map(|index| {
            let mut point = windows_sys::Win32::Foundation::POINT::default();
            let ok = super::bypass(|| unsafe {
                SendMessageW(
                    view,
                    LVM_GETITEMPOSITION,
                    index as usize,
                    (&raw mut point) as isize,
                )
            });
            (ok != 0).then_some((index as i32, point))
        })
        .collect()
}

/// Reconcile before native painting, so a same-count reorder cannot paint a collected icon.
pub(super) fn refresh_identities(view: HWND) {
    let active = STATE.with(|s| s.borrow().as_ref().is_some_and(|s| s.identities.active));
    if !active {
        return;
    }
    let count = unsafe { SendMessageW(view, LVM_GETITEMCOUNT, 0, 0) };
    if !(0..=crate::protocol::MAX_LAYOUT_ITEMS as isize).contains(&count) {
        return;
    }
    let names: Option<BTreeMap<_, _>> = (0..count)
        .map(|i| name_hash(view, i as i32).map(|h| (i as i32, h)))
        .collect();
    let Some(names) = names else {
        return;
    };
    if STATE.with(|s| {
        s.borrow()
            .as_ref()
            .is_some_and(|s| s.identities.expected == names)
    }) {
        return;
    }
    // Read the underlying arrangement outside STATE's borrow: these synchronous
    // control messages re-enter the geometry subclass. Never write Shell positions.
    let slots = original_slots(view, count);
    let mut origin = windows_sys::Win32::Foundation::POINT::default();
    unsafe {
        windows_sys::Win32::Graphics::Gdi::ClientToScreen(view, &raw mut origin);
    }
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        let cache = &mut state.identities;
        if names == cache.expected {
            return;
        }
        state.cache.layout_changed();
        let mut counts = BTreeMap::new();
        for hash in names.values() {
            *counts.entry(*hash).or_insert(0usize) += 1;
        }
        cache.current_hidden = names
            .iter()
            .filter(|(_, h)| cache.unique_hidden.contains(h) && counts.get(h) == Some(&1))
            .map(|(i, _)| *i)
            .collect();
        state.targets.clear();
        state.pending = None;
        state.members.clear();
        if let Some(slots) = slots {
            let mut monitors: BTreeMap<isize, Vec<_>> = BTreeMap::new();
            for (index, point) in slots {
                let screen = windows_sys::Win32::Foundation::POINT {
                    x: point.x.saturating_add(origin.x),
                    y: point.y.saturating_add(origin.y),
                };
                let monitor = unsafe {
                    windows_sys::Win32::Graphics::Gdi::MonitorFromPoint(
                        screen,
                        windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
                    )
                };
                monitors
                    .entry(monitor as isize)
                    .or_default()
                    .push((index, point));
            }
            for slots in monitors.values_mut() {
                slots.sort_by_key(|(_, p)| (p.x, p.y));
                for ((index, _), (_, destination)) in slots
                    .iter()
                    .filter(|(i, _)| !cache.current_hidden.contains(i))
                    .zip(slots.iter())
                {
                    state.targets.insert(*index, *destination);
                }
            }
        }
        if !cache.notified {
            cache.notified = true;
            unsafe {
                windows_sys::Win32::Graphics::Gdi::InvalidateRect(view, std::ptr::null(), 0);
                PostMessageW(
                    cache.owner as HWND,
                    crate::protocol::SCENE_DIRTY_MESSAGE,
                    0,
                    0,
                );
            }
        }
    });
    // A same-count reorder can turn a selected visible index into a hidden one.
    // Clean up only after releasing STATE: querying native state re-enters us.
    clear_selection(view);
}
pub(super) fn is_hidden(item: i32) -> bool {
    STATE.with(|s| {
        s.borrow().as_ref().is_some_and(|s| {
            if s.identities.active {
                s.identities.current_hidden.contains(&item)
            } else {
                s.members.get(&item) == Some(&(crate::protocol::HIDDEN_ITEM as usize - 1))
            }
        })
    })
}
pub(super) fn clear_selection(view: HWND) {
    if MENU.get() || CLEANING.replace(true) {
        return;
    }
    let hidden: Vec<_> = STATE.with(|s| {
        s.borrow().as_ref().map_or_else(Vec::new, |s| {
            if s.identities.active {
                return s.identities.current_hidden.iter().copied().collect();
            }
            s.members
                .iter()
                .filter(|(_, p)| **p == crate::protocol::HIDDEN_ITEM as usize - 1)
                .map(|(i, _)| *i)
                .collect()
        })
    });
    for item in hidden {
        // A no-op state write still enters the native selection machinery. In
        // particular, do not send these while it is processing a visible press.
        let selected = unsafe {
            SendMessageW(view, LVM_GETITEMSTATE, item as usize,
                (LVIS_SELECTED | LVIS_FOCUSED) as isize)
        };
        if selected == 0 {
            continue;
        }
        let state = LVITEMW {
            stateMask: LVIS_SELECTED | LVIS_FOCUSED,
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                view,
                LVM_SETITEMSTATE,
                item as usize,
                (&raw const state) as isize,
            );
        }
    }
    CLEANING.set(false);
}
pub(super) unsafe fn notification(header: &NMHDR, lp: isize) -> Option<isize> {
    let ours = STATE.with(|s| {
        s.borrow()
            .as_ref()
            .is_some_and(|s| s.view == header.hwndFrom)
    });
    if ours && MENU.get() && [LVN_BEGINLABELEDITW, LVN_BEGINLABELEDITA].contains(&header.code) {
        // Both notification encodings have the same item-index prefix. Do not read text.
        let edit = unsafe { &*(lp as *const NMLVDISPINFOW) };
        if is_hidden(edit.item.iItem) {
            RENAME_REQUESTED.set(true);
            return Some(1); // Shell must not create an editor at the hidden desktop slot.
        }
    }
    if !ours || MENU.get() || CLEANING.get() {
        return None;
    }
    if header.code == LVN_ITEMCHANGING || header.code == LVN_ITEMCHANGED {
        let change = unsafe { &*(lp as *const NMLISTVIEW) };
        super::cache::geometry_changed((change.iItem >= 0).then_some(change.iItem));
    } else if header.code == LVN_ODSTATECHANGED {
        super::cache::geometry_changed(None);
    }
    if header.code == LVN_ITEMCHANGING {
        let change = unsafe { &*(lp as *const NMLISTVIEW) };
        if is_hidden(change.iItem) && change.uNewState & (LVIS_SELECTED | LVIS_FOCUSED) != 0 {
            return Some(1);
        }
    }
    let hidden_selected = if header.code == LVN_ITEMCHANGED {
        let change = unsafe { &*(lp as *const NMLISTVIEW) };
        change.uChanged & LVIF_STATE != 0
            && change.uNewState & (LVIS_SELECTED | LVIS_FOCUSED) != 0
            && (change.iItem < 0 || is_hidden(change.iItem))
    } else if header.code == LVN_ODSTATECHANGED {
        let change = unsafe { &*(lp as *const NMLVODSTATECHANGE) };
        change.uNewState & (LVIS_SELECTED | LVIS_FOCUSED) != 0
            && (change.iFrom..=change.iTo).any(is_hidden)
    } else { false };
    if hidden_selected {
        clear_selection(header.hwndFrom);
    }
    None
}
#[allow(clippy::too_many_lines)]
pub(super) unsafe fn message(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> Option<isize> {
    if msg == WM_NCDESTROY {
        MENU.set(false);
    }
    if msg == WM_KEYDOWN || msg == WM_LBUTTONDOWN {
        refresh_identities(hwnd);
    }
    if MENU.get() || CLEANING.get() {
        return None;
    }
    let active = STATE.with(|s| {
        s.borrow().as_ref().is_some_and(|s| {
            s.identities.active
                || s.members
                    .values()
                    .any(|p| *p == crate::protocol::HIDDEN_ITEM as usize - 1)
        })
    });
    if !active {
        return None;
    }
    if msg == LVM_SETITEMSTATE && lp != 0 {
        let state = unsafe { &*(lp as *const LVITEMW) };
        if state.state & state.stateMask & (LVIS_SELECTED | LVIS_FOCUSED) != 0 {
            if wp == usize::MAX {
                let count = unsafe { SendMessageW(hwnd, LVM_GETITEMCOUNT, 0, 0) };
                for item in 0..count {
                    if !is_hidden(item as i32) {
                        unsafe {
                            DefSubclassProc(hwnd, msg, item as usize, lp);
                        }
                    }
                }
                return Some(1);
            }
            if is_hidden(wp as i32) {
                return Some(1);
            }
        }
    }
    if msg == WM_KEYDOWN {
        if wp == usize::from(b'A') && unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0 {
            let state = LVITEMW {
                stateMask: LVIS_SELECTED,
                state: LVIS_SELECTED,
                ..Default::default()
            };
            unsafe {
                SendMessageW(
                    hwnd,
                    LVM_SETITEMSTATE,
                    usize::MAX,
                    (&raw const state) as isize,
                );
            }
            return Some(0);
        }
        // Native spatial navigation can land on an owner-data item even when it is not painted.
        if [VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN, VK_HOME, VK_END]
            .iter()
            .any(|k| usize::from(*k) == wp)
        {
            let count = unsafe { SendMessageW(hwnd, LVM_GETITEMCOUNT, 0, 0) };
            let mut items = Vec::new();
            for item in 0..count {
                if is_hidden(item as i32) {
                    continue;
                }
                let mut p = windows_sys::Win32::Foundation::POINT::default();
                unsafe {
                    SendMessageW(
                        hwnd,
                        LVM_GETITEMPOSITION,
                        item as usize,
                        (&raw mut p) as isize,
                    );
                }
                items.push((item, p));
            }
            items.sort_by_key(|(_, p)| (p.x, p.y));
            let current =
                unsafe { SendMessageW(hwnd, LVM_GETNEXTITEM, usize::MAX, LVNI_FOCUSED as isize) };
            let origin = items.iter().find(|(i, _)| *i == current).map(|(_, p)| *p);
            let target = if wp == usize::from(VK_END) {
                items.last()
            } else if wp == usize::from(VK_HOME) || origin.is_none() {
                items.first()
            } else {
                let p = origin.unwrap();
                items
                    .iter()
                    .filter_map(|(i, q)| {
                        let (along, across) = match wp as u16 {
                            VK_LEFT => (p.x - q.x, (p.y - q.y).abs()),
                            VK_RIGHT => (q.x - p.x, (p.y - q.y).abs()),
                            VK_UP => (p.y - q.y, (p.x - q.x).abs()),
                            _ => (q.y - p.y, (p.x - q.x).abs()),
                        };
                        (along > 0).then_some(((i64::from(across) * 10000 + i64::from(along)), *i))
                    })
                    .min()
                    .and_then(|(_, i)| items.iter().find(|(j, _)| *j == i))
            };
            if let Some((item, _)) = target {
                let keep = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0
                    || unsafe { GetKeyState(i32::from(VK_SHIFT)) } < 0;
                let clear = LVITEMW {
                    stateMask: LVIS_FOCUSED | if keep { 0 } else { LVIS_SELECTED },
                    ..Default::default()
                };
                unsafe {
                    SendMessageW(
                        hwnd,
                        LVM_SETITEMSTATE,
                        usize::MAX,
                        (&raw const clear) as isize,
                    );
                }
                let select = LVITEMW {
                    stateMask: LVIS_SELECTED | LVIS_FOCUSED,
                    state: LVIS_FOCUSED
                        | if unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0 {
                            0
                        } else {
                            LVIS_SELECTED
                        },
                    ..Default::default()
                };
                unsafe {
                    SendMessageW(
                        hwnd,
                        LVM_SETITEMSTATE,
                        *item as usize,
                        (&raw const select) as isize,
                    );
                }
            }
            return Some(0);
        }
        clear_selection(hwnd);
    }
    None
}
