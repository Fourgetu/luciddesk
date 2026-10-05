//! Listen-only low-level keyboard hook.
//!
//! Windows reserves some combinations that `RegisterHotKey` can never claim;
//! `Win + Space` is the input-method switcher and fails with
//! `ERROR_HOTKEY_ALREADY_REGISTERED`. To still honour such a binding the runtime
//! installs a `WH_KEYBOARD_LL` hook that observes the combination and posts the
//! same notification the registered-hotkey path uses.
//!
//! The hook never swallows input: every event is forwarded with
//! `CallNextHookEx`, so Windows keeps handling the combination as well. It is
//! installed only while at least one binding needs it, and removed as soon as
//! the last such binding is cleared.

use std::cell::{Cell, RefCell};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU,
    VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, KBDLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// One binding served by the hook, with the window that must receive the
/// `WM_HOTKEY` notification and the id the message loop already switches on.
#[derive(Clone, Copy)]
struct Watched {
    hwnd: HWND,
    key: u16,
    modifiers: u8,
    notify_id: usize,
}

thread_local! {
    /// Bindings served by the hook. The search and show-panels shortcuts can
    /// both fall back here, so this is a list rather than a single slot.
    static WATCHED: RefCell<Vec<Watched>> = const { RefCell::new(Vec::new()) };
    static HOOK: Cell<*mut core::ffi::c_void> = const { Cell::new(std::ptr::null_mut()) };
    /// Modifier state accumulated from hook events, because the callback can
    /// run before the system updates `GetAsyncKeyState`.
    static MODS: Cell<u8> = const { Cell::new(0) };
    /// Notification ids already fired for the current key hold, so one press
    /// fires each bound action exactly once.
    static FIRED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// Bit for a modifier virtual key, or 0 when the key is not a modifier.
fn modifier_bit(vk: u32) -> u8 {
    let vk = vk as u16;
    if matches!(vk, VK_CONTROL | VK_LCONTROL | VK_RCONTROL) {
        1
    } else if matches!(vk, VK_SHIFT | VK_LSHIFT | VK_RSHIFT) {
        2
    } else if matches!(vk, VK_MENU | VK_LMENU | VK_RMENU) {
        4
    } else if matches!(vk, VK_LWIN | VK_RWIN) {
        8
    } else {
        0
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && lparam != 0 {
        // SAFETY: the system passes a valid KBDLLHOOKSTRUCT for a keyboard
        // hook, and it stays alive for the duration of this call.
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let down = wparam == WM_KEYDOWN as usize || wparam == WM_SYSKEYDOWN as usize;
        let up = wparam == WM_KEYUP as usize || wparam == WM_SYSKEYUP as usize;
        let vk = event.vkCode;

        let bit = modifier_bit(vk);
        if bit != 0 {
            if down || up {
                MODS.with(|mods| {
                    let current = mods.get();
                    mods.set(if down { current | bit } else { current & !bit });
                });
            }
        } else if down {
            let pressed = MODS.with(Cell::get);
            WATCHED.with(|watched| {
                for binding in watched.borrow().iter() {
                    if u32::from(binding.key) != vk
                        || binding.modifiers == 0
                        || binding.modifiers != pressed
                    {
                        continue;
                    }
                    let first = FIRED.with(|fired| {
                        let mut fired = fired.borrow_mut();
                        if fired.contains(&binding.notify_id) {
                            false
                        } else {
                            fired.push(binding.notify_id);
                            true
                        }
                    });
                    if first {
                        // SAFETY: posting a notification is inert; the target
                        // window belongs to this thread's message loop. Reuse
                        // the id the registered-hotkey path already dispatches.
                        unsafe {
                            PostMessageW(binding.hwnd, WM_HOTKEY, binding.notify_id, 0);
                        }
                    }
                }
            });
        } else if up {
            // Releasing the trigger key re-arms it for the next press.
            WATCHED.with(|watched| {
                let rearm = watched
                    .borrow()
                    .iter()
                    .any(|binding| u32::from(binding.key) == vk);
                if rearm {
                    FIRED.with(|fired| fired.borrow_mut().clear());
                }
            });
        }
    }
    // Never swallow: Windows continues to handle the combination itself.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// Registers `notify_id` as a hook-served binding, installing the hook on first
/// use. Returns `true` when the binding is being watched, so the caller can
/// report it as active even though `RegisterHotKey` refused it.
pub(super) fn watch(hwnd: HWND, notify_id: usize, key: u16, modifiers: u8) -> bool {
    if hwnd.is_null() {
        return false;
    }
    // Install first: a failed hook must not leave a binding recorded as served.
    if HOOK.with(Cell::get).is_null() {
        // SAFETY: a global low-level keyboard hook whose callback matches
        // HOOKPROC, using this executable's module handle.
        let hook = unsafe {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), GetModuleHandleW(std::ptr::null()), 0)
        };
        if hook.is_null() {
            return false;
        }
        HOOK.with(|h| h.set(hook));
    }
    WATCHED.with(|watched| {
        let mut watched = watched.borrow_mut();
        watched.retain(|binding| binding.notify_id != notify_id);
        watched.push(Watched {
            hwnd,
            key,
            modifiers,
            notify_id,
        });
    });
    FIRED.with(|fired| fired.borrow_mut().clear());
    true
}

/// Stops serving `notify_id`. The hook is removed once nothing else needs it.
pub(super) fn clear(notify_id: usize) {
    let empty = WATCHED.with(|watched| {
        let mut watched = watched.borrow_mut();
        watched.retain(|binding| binding.notify_id != notify_id);
        watched.is_empty()
    });
    FIRED.with(|fired| fired.borrow_mut().clear());
    if !empty {
        return;
    }
    MODS.with(|mods| mods.set(0));
    let hook = HOOK.with(Cell::get);
    if !hook.is_null() {
        // SAFETY: the handle came from SetWindowsHookExW above and is removed
        // exactly once because HOOK is cleared immediately after.
        unsafe {
            UnhookWindowsHookEx(hook);
        }
        HOOK.with(|h| h.set(std::ptr::null_mut()));
    }
}
