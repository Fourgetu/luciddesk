//! OLE drop registration, drag previews, and temporary drop descriptions.
mod description;
pub(super) mod image;
pub(super) mod target;

/// Whether a drag that started in a desktop pane should be handed to OLE because the
/// pointer has moved to another program.
///
/// Desktop panes reorder and regroup internally, so the hand-off only happens once the
/// pointer is outside every one of our panes and above a real foreign window. Anything
/// the Shell or our own process owns stays internal: the desktop accepts our membership
/// drops, the taskbar is not a drop target, and floating overlays (screenshot tools,
/// HUDs) must not hijack a drag that is merely passing over them. Restricting the
/// hand-off to Explorer windows, as this previously did, made dropping onto mail, IM or
/// chat clients impossible.
pub(super) fn over_foreign_window(
    pane: windows_sys::Win32::Foundation::HWND,
    point: windows_sys::Win32::Foundation::POINT,
) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, WindowFromPoint};
    unsafe {
        let target = GetAncestor(WindowFromPoint(point), GA_ROOT);
        if target.is_null() {
            return false;
        }

        hands_off(&classify(pane, target, point))
    }
}

/// Everything the decision needs, gathered from one candidate window.
struct Target {
    name: String,
    topmost: bool,
    tool: bool,
    layered: bool,
    owned: bool,
    same_process: bool,
    over_own_pane: bool,
}

/// Pure decision, so the rules are testable without moving a real pointer.
fn hands_off(target: &Target) -> bool {
    if target.same_process || target.over_own_pane || is_internal_surface(&target.name) {
        return false;
    }
    // A floating overlay is a tool, layered or owned topmost window rather than an
    // application the user could drop files into.
    !(target.topmost && (target.tool || target.layered || target.owned))
}

/// Window classes the Shell owns plus our own window class.
fn is_internal_surface(class: &str) -> bool {
    matches!(
        class,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "SysShadow"
            | "windows-window.Window"
    )
}

fn classify(
    pane: windows_sys::Win32::Foundation::HWND,
    target: windows_sys::Win32::Foundation::HWND,
    point: windows_sys::Win32::Foundation::POINT,
) -> Target {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetClassNameW, GetWindow, GetWindowLongW,
    };
    const GW_OWNER: u32 = 4;
    const WS_EX_TOPMOST: i32 = 0x0000_0008;
    const WS_EX_TOOLWINDOW: i32 = 0x0000_0080;
    const WS_EX_LAYERED: i32 = 0x0008_0000;
    unsafe {
        let mut name = [0u16; 64];
        let length = GetClassNameW(target, name.as_mut_ptr(), name.len() as i32).max(0) as usize;
        let ex = GetWindowLongW(target, GWL_EXSTYLE);
        Target {
            name: String::from_utf16_lossy(&name[..length.min(name.len())]),
            topmost: ex & WS_EX_TOPMOST != 0,
            tool: ex & WS_EX_TOOLWINDOW != 0,
            layered: ex & WS_EX_LAYERED != 0,
            owned: !GetWindow(target, GW_OWNER).is_null(),
            same_process: process_of(target) == process_of(pane),
            over_own_pane: over_own_pane(point),
        }
    }
}

/// Whether the point still belongs to one of this process's pane windows.
///
/// The decision must not be a rectangle test around every pane: desktop panes tile the
/// work area, so their union covers the whole screen and nothing would ever leave them.
/// What matters is whether the window under the pointer is one of ours.
fn over_own_pane(point: windows_sys::Win32::Foundation::POINT) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, WindowFromPoint};
    unsafe {
        let target = GetAncestor(WindowFromPoint(point), GA_ROOT);
        !target.is_null() && process_of(target) == std::process::id()
    }
}

fn process_of(window: windows_sys::Win32::Foundation::HWND) -> u32 {
    let mut process = 0;
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            window,
            &raw mut process,
        );
    }
    process
}

#[cfg(test)]
mod tests {
    use super::{Target, hands_off, is_internal_surface};

    fn target(class: &str) -> Target {
        Target {
            name: class.to_owned(),
            topmost: false,
            tool: false,
            layered: false,
            owned: false,
            same_process: false,
            over_own_pane: false,
        }
    }

    #[test]
    fn internal_surfaces_stay_on_the_membership_path() {
        for class in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
            "SysShadow",
            "windows-window.Window",
        ] {
            assert!(is_internal_surface(class), "{class} must stay internal");
            assert!(!hands_off(&target(class)), "{class} must not hand off");
        }
    }

    #[test]
    fn other_programs_receive_the_drag() {
        // Chat clients, mail clients and Explorer windows all accept an OLE file drop.
        for class in [
            "CabinetWClass",
            "Qt51514QWindowIcon",
            "Chrome_WidgetWin_1",
            "OpusApp",
            "ApplicationFrameWindow",
        ] {
            assert!(!is_internal_surface(class), "{class} must be a target");
            assert!(hands_off(&target(class)), "{class} must hand off");
        }
    }

    #[test]
    fn our_own_windows_and_panes_never_hand_off() {
        let mut own = target("Chrome-like-foreign");
        own.same_process = true;
        assert!(!hands_off(&own), "another window of ours is still internal");
        let mut in_pane = target("Chrome-like-foreign");
        in_pane.over_own_pane = true;
        assert!(!hands_off(&in_pane), "the gap between panes is still internal");
    }

    #[test]
    fn floating_overlays_do_not_hijack_a_passing_drag() {
        // Screenshot tools and HUDs are topmost and either tool, layered or owned.
        for (tool, layered, owned) in [(true, true, false), (false, false, true), (true, false, false)] {
            let mut overlay = target("HwndWrapper[VibeGauge]");
            overlay.topmost = true;
            overlay.tool = tool;
            overlay.layered = layered;
            overlay.owned = owned;
            assert!(!hands_off(&overlay), "overlay must not take the drag");
        }
        // A plain always-on-top application window is still a valid drop target.
        let mut pinned = target("Qt51514QWindowIcon");
        pinned.topmost = true;
        assert!(hands_off(&pinned), "an always-on-top chat window still accepts files");
    }
}

