//! Bounded, pointer-free request payload for `WM_COPYDATA`.
use windows_sys::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW;

pub const VERSION: u32 = 2;
pub const MAGIC: usize = 0x4c50_484b;
pub const MAX_AREAS: usize = 16;
pub const OK: isize = 0x4c50;
/// Menu completed with a request to rename a hidden pane item in the controller.
pub const RENAME_REQUESTED: isize = OK + 1;
pub const REJECTED: isize = -1;
pub const QUERY: u32 = 1;
pub const SET_AREAS: u32 = 2;
pub const MOVE_ITEM: u32 = 3;
pub const DETACH: u32 = 4;
pub const QUERY_AUTOARRANGE: u32 = 5;
pub const QUERY_AREA_COUNT: u32 = 6;
pub const QUERY_ITEM_COUNT: u32 = 7;
pub const QUERY_ITEM_AREA: u32 = 8;
pub const QUERY_GENERATION: u32 = 9;
pub const QUERY_ICON_RECT: u32 = 10;
pub const QUERY_HIT: u32 = 11;
pub const CLEAR_POSITIONS: u32 = 12;
pub const BEGIN_POSITIONS: u32 = 13;
pub const COMMIT_POSITIONS: u32 = 14;
pub const QUERY_ORIGINAL_POSITION: u32 = 15;
/// Native inventory/layout changes only; publishing our geometry does not invalidate caches.
pub const QUERY_SHELL_GENERATION: u32 = 16;
pub const MENU_SELECTION_BEGIN: u32 = 17;
pub const MENU_SELECTION_END: u32 = 18;
pub const QUERY_MOVE_REQUESTS: u32 = 19;
pub const QUERY_DROP_PROXY: u32 = 20;
pub const QUERY_INSERTION_TARGET: u32 = 21;
/// Pane input relinquishes the desktop's selected and keyboard-focused items.
pub const CLEAR_DESKTOP_SELECTION: u32 = 22;
/// Pointer-free queued notification; unlike WM_COPYDATA it never waits on Explorer.
pub fn clear_selection_message() -> u32 {
    static MESSAGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe {
        RegisterWindowMessageW(windows_sys::w!("LucidPane.ClearDesktopSelection.v1"))
    })
}
/// Item remains in Shell inventory, but has no desktop presentation or input target.
pub const HIDDEN_ITEM: u32 = u32::MAX;
pub const SCENE_DIRTY_MESSAGE: u32 = 0x8000 + 0x4a0;
/// A real desktop input gesture; wParam is the ListView HWND, lParam is its
/// GetMessageTime tick (u32). Receivers must reject events older than pane input.
pub const DESKTOP_INPUT_MESSAGE: u32 = 0x8000 + 0x4a1;

pub const LAYOUT_MAGIC: usize = MAGIC + 2;
pub const MAX_LAYOUT_ITEMS: usize = 512;
pub const TEXTURE_MAGIC: usize = MAGIC + 3;
pub const BASELINE_MAGIC: usize = MAGIC + 4;

/// Compare all original coordinates in one read-only cross-process request.
#[repr(C)]
pub struct BaselineCheck {
    pub version: u32,
    pub generation: u32,
    pub count: u32,
    pub items: [ItemPosition; MAX_LAYOUT_ITEMS],
}
impl BaselineCheck {
    #[must_use]
    pub fn valid(&self) -> bool {
        self.version == VERSION
            && self.count as usize <= MAX_LAYOUT_ITEMS
            && self.items[..self.count as usize]
                .iter()
                .all(|item| item.item >= 0)
    }

    pub fn matches(
        &self,
        generation: u32,
        mut position: impl FnMut(i32) -> Option<(i32, i32)>,
    ) -> bool {
        self.valid()
            && self.generation == generation
            && self.items[..self.count as usize]
                .iter()
                .all(|item| position(item.item) == Some((item.x, item.y)))
    }
}
pub const PANE_HEADER: i32 = 48;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct PaneAppearance {
    pub bounds: Area,
    pub material: u32,
    pub radius: i32,
    pub title: [u16; 96],
}

impl Default for PaneAppearance {
    fn default() -> Self {
        Self {
            bounds: Area::default(),
            material: 0,
            radius: 12,
            title: [0; 96],
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct TextureHeader {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub bounds: Area,
}

impl TextureHeader {
    #[must_use]
    pub fn byte_count(&self) -> Option<usize> {
        if self.version != VERSION
            || self.width == 0
            || self.height == 0
            || self.width > 2048
            || self.height > 2048
            || !self.bounds.valid()
        {
            return None;
        }
        Some(self.width as usize * self.height as usize * 4)
    }
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct ItemPosition {
    pub name_hash: u64,
    pub item: i32,
    pub x: i32,
    pub y: i32,
    pub reserved: u32,
}

/// One bounded transaction, copied by Windows; no remote pointers or unbounded allocation.
#[repr(C)]
pub struct LayoutBatch {
    pub areas: Request,
    pub count: u32,
    pub flush: u32,
    pub pane_count: u32,
    pub panes: [PaneAppearance; MAX_AREAS],
    pub items: [ItemPosition; MAX_LAYOUT_ITEMS],
}

impl LayoutBatch {
    #[must_use]
    pub fn valid(&self) -> bool {
        self.areas.command == SET_AREAS
            && self.areas.valid()
            && self.count as usize <= MAX_LAYOUT_ITEMS
            && self.flush <= 1
            && self.pane_count as usize <= MAX_AREAS
            && self.panes[..self.pane_count as usize].iter().all(|p| {
                p.bounds.valid()
                    && [p.bounds.left, p.bounds.top, p.bounds.right, p.bounds.bottom]
                        .iter()
                        .all(|c| c.abs_diff(0) <= 100_000)
                    && p.bounds.bottom - p.bounds.top > PANE_HEADER
                    && p.material <= 1
                    && p.bounds.right - p.bounds.left > 2 * p.radius
                    && p.bounds.bottom - p.bounds.top > 2 * p.radius
                    && (0..=32).contains(&p.radius)
                    && p.title[95] == 0
            })
            && self.items[..self.count as usize].iter().all(|i| {
                i.item >= 0
                    && (i.reserved <= self.pane_count || i.reserved == HIDDEN_ITEM)
                    && i.x.abs_diff(0) <= 100_000
                    && i.y.abs_diff(0) <= 100_000
            })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Area {
    #[must_use]
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    #[must_use]
    pub fn valid(self) -> bool {
        self.left < self.right && self.top < self.bottom
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Request {
    pub version: u32,
    pub command: u32,
    pub name_hash: u64,
    pub count: u32,
    pub item: i32,
    pub x: i32,
    pub y: i32,
    pub areas: [Area; MAX_AREAS],
}

impl Request {
    #[must_use]
    pub fn new(command: u32) -> Self {
        Self {
            version: VERSION,
            command,
            name_hash: 0,
            count: 0,
            item: -1,
            x: 0,
            y: 0,
            areas: [Area::default(); MAX_AREAS],
        }
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        self.version == VERSION
            && (QUERY..=CLEAR_DESKTOP_SELECTION).contains(&self.command)
            && self.count as usize <= MAX_AREAS
            && (self.command != SET_AREAS
                || (self.count > 0 && self.areas[..self.count as usize].iter().all(|a| a.valid())))
            && (self.command != MOVE_ITEM || self.item >= 0)
    }
}

#[must_use]
pub fn name_hash(name: impl IntoIterator<Item = u16>) -> u64 {
    name.into_iter().fold(0xcbf2_9ce4_8422_2325, |hash, c| {
        (hash ^ u64::from(c)).wrapping_mul(0x100_0000_01b3)
    })
}

#[must_use]
pub fn geometry_attach_message() -> u32 {
    unsafe { RegisterWindowMessageW(windows_sys::w!("LucidPane.DesktopHook.Geometry.Attach.v2")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_check_validates_bounds_generation_and_signed_coordinates() {
        let mut check = BaselineCheck {
            version: VERSION,
            generation: 7,
            count: 2,
            items: [ItemPosition::default(); MAX_LAYOUT_ITEMS],
        };
        check.items[0] = ItemPosition {
            item: 2,
            x: -1,
            y: -100,
            ..Default::default()
        };
        check.items[1] = ItemPosition {
            item: 5,
            x: 200,
            y: 300,
            ..Default::default()
        };
        let read = |item| match item {
            2 => Some((-1, -100)),
            5 => Some((200, 300)),
            _ => None,
        };
        assert!(check.matches(7, read));
        assert!(!check.matches(8, |_| panic!("stale generations must not scan coordinates")));
        assert!(!check.matches(7, |_| None));
        check.items[1].y += 1;
        assert!(!check.matches(7, read));
        check.count = MAX_LAYOUT_ITEMS as u32 + 1;
        assert!(!check.matches(7, |_| panic!("invalid counts must not index the buffer")));
        check.count = 1;
        check.items[0].item = -1;
        assert!(!check.valid());
        check.count = 0;
        assert!(check.matches(7, |_| panic!("empty desktop needs no coordinate reads")));
        check.version += 1;
        assert!(!check.valid());
    }

    #[test]
    fn rejects_unbounded_protocol_payloads() {
        let mut r = Request::new(SET_AREAS);
        r.count = 17;
        assert!(!r.valid());
        r.count = 1;
        assert!(!r.valid());
        r.areas[0] = Area {
            left: 0,
            top: 0,
            right: 10,
            bottom: 10,
        };
        assert!(r.valid());
        r.version += 1;
        assert!(!r.valid());
    }
}
