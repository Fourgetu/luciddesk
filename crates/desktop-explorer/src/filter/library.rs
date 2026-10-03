//! Reference-counted DLL lifetime: callbacks finish on the Explorer STA and menu
//! threads finish their Rust entry trampolines before a native thread releases its ref.
use std::{
    ptr::null_mut,
    sync::atomic::{AtomicUsize, Ordering},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, HMODULE, HWND},
    System::{LibraryLoader::*, Threading::*},
    UI::WindowsAndMessaging::*,
};

static CALLBACKS: AtomicUsize = AtomicUsize::new(0);
pub(super) struct Callback;
impl Callback {
    pub fn enter() -> Self {
        CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for Callback {
    fn drop(&mut self) {
        CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn module() -> windows::core::Result<HMODULE> {
    let mut module = null_mut();
    if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            (super::LucidDeskFilterHook as *const ()).cast(),
            &raw mut module,
        )
    } == 0
    {
        return Err(windows::core::Error::from_thread());
    }
    Ok(module)
}

struct Release {
    module: HMODULE,
    wait: HANDLE,
    barrier: HWND,
}
unsafe extern "system" fn release(raw: *mut core::ffi::c_void) -> u32 {
    let state = unsafe { Box::from_raw(raw.cast::<Release>()) };
    unsafe {
        WaitForSingleObject(state.wait, INFINITE);
        CloseHandle(state.wait);
    }
    if !state.barrier.is_null() {
        // A zero callback count alone leaves an epilogue race. A sent message to
        // this system-owned STATIC window also waits for the STA to leave that stack.
        // Every DLL callback frame, including reentrant frames, contributes to the count.
        loop {
            while CALLBACKS.load(Ordering::Acquire) != 0 {
                unsafe {
                    Sleep(1);
                }
            }
            let mut result = 0;
            if unsafe {
                SendMessageTimeoutW(
                    state.barrier,
                    WM_NULL,
                    0,
                    0,
                    SMTO_BLOCK | SMTO_ABORTIFHUNG,
                    1000,
                    &raw mut result,
                )
            } != 0
                && CALLBACKS.load(Ordering::Acquire) == 0
            {
                break;
            }
            // An unresponsive Explorer retains this ordinary reference rather than
            // unloading code that may still be executing. No thread is terminated.
            unsafe {
                Sleep(10);
            }
        }
        let mut result = 0;
        unsafe {
            SendMessageTimeoutW(
                state.barrier,
                WM_CLOSE,
                0,
                0,
                SMTO_BLOCK | SMTO_ABORTIFHUNG,
                1000,
                &raw mut result,
            );
        }
    }
    let module = state.module;
    drop(state);
    // There must be no Rust thread trampoline or DLL return address after unload.
    unsafe {
        FreeLibraryAndExitThread(module, 0);
    }
}

fn reaper(state: Release) -> windows::core::Result<()> {
    let state = Box::into_raw(Box::new(state));
    let thread = unsafe { CreateThread(null_mut(), 0, Some(release), state.cast(), 0, null_mut()) };
    if thread.is_null() {
        let error = windows::core::Error::from_thread();
        let state = unsafe { Box::from_raw(state) };
        unsafe {
            CloseHandle(state.wait);
        }
        // The caller still owns live callbacks or a running thread. Keeping this
        // reference on this exceptional path is safer than unloading beneath it.
        return Err(error);
    }
    unsafe {
        CloseHandle(thread);
    }
    Ok(())
}

pub(super) struct StaLease(HANDLE);
impl StaLease {
    pub fn new() -> windows::core::Result<Self> {
        let module = module()?;
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                windows_sys::w!("STATIC"),
                windows_sys::w!("LucidDesk DLL lifecycle"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if window.is_null() {
            let error = windows::core::Error::from_thread();
            unsafe {
                windows_sys::Win32::Foundation::FreeLibrary(module);
            }
            return Err(error);
        }
        let event = unsafe { CreateEventW(null_mut(), 1, 0, null_mut()) };
        let mut wait = null_mut();
        if event.is_null()
            || unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    event,
                    GetCurrentProcess(),
                    &raw mut wait,
                    0,
                    0,
                    DUPLICATE_SAME_ACCESS,
                )
            } == 0
        {
            let error = windows::core::Error::from_thread();
            unsafe {
                DestroyWindow(window);
                if !event.is_null() {
                    CloseHandle(event);
                }
                windows_sys::Win32::Foundation::FreeLibrary(module);
            }
            return Err(error);
        }
        if let Err(error) = reaper(Release {
            module,
            wait,
            barrier: window,
        }) {
            unsafe {
                DestroyWindow(window);
                CloseHandle(event);
            }
            return Err(error);
        }
        Ok(Self(event))
    }
}
impl Drop for StaLease {
    fn drop(&mut self) {
        unsafe {
            SetEvent(self.0);
            CloseHandle(self.0);
        }
    }
}

pub(super) fn keep_thread(thread: &std::thread::JoinHandle<()>) -> windows::core::Result<()> {
    use std::os::windows::io::AsRawHandle;
    let module = module()?;
    let mut wait = null_mut();
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            thread.as_raw_handle().cast(),
            GetCurrentProcess(),
            &raw mut wait,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        // The menu thread has already started, so retain the module on failure.
        return Err(windows::core::Error::from_thread());
    }
    reaper(Release {
        module,
        wait,
        barrier: null_mut(),
    })
}
