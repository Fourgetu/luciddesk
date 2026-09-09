//! Bounded, pointer-free request payload for `WM_COPYDATA`.
use windows_sys::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW;

pub const VERSION: u32 = 1;
pub const MAGIC: usize = 0x4c50_484b;
pub const MAX_AREAS: usize = 16;
pub const OK: isize = 0x4c50;
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
pub const QUERY_BASELINE_COORD: u32 = 10;
pub const QUERY_ICON_RECT: u32 = 11;
pub const QUERY_HIT: u32 = 12;
pub const CLEAR_POSITIONS: u32 = 13;
pub const BEGIN_POSITIONS: u32 = 14;
pub const COMMIT_POSITIONS: u32 = 15;
pub const QUERY_ORIGINAL_POSITION: u32 = 16;
/// Native inventory/layout changes only; publishing our geometry does not invalidate caches.
pub const QUERY_SHELL_GENERATION: u32 = 17;
pub const MENU_SELECTION_BEGIN: u32 = 18;
pub const MENU_SELECTION_END: u32 = 19;
pub const QUERY_MOVE_REQUESTS: u32 = 20;
pub const QUERY_DROP_PROXY: u32 = 21;
pub const QUERY_INSERTION_TARGET: u32 = 22;
/// Item remains in Shell inventory, but has no desktop presentation or input target.
pub const HIDDEN_ITEM: u32 = u32::MAX;
pub const SCENE_DIRTY_MESSAGE: u32 = 0x8000 + 0x4a0;
pub const LAYOUT_MAGIC: usize = MAGIC + 2;
pub const MAX_LAYOUT_ITEMS: usize = 512;
pub const TEXTURE_MAGIC: usize = MAGIC + 3;
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

    fn intersect(self, other: Self) -> Option<Self> {
        let result = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        result.valid().then_some(result)
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
            && (QUERY..=QUERY_INSERTION_TARGET).contains(&self.command)
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
pub fn attach_message() -> u32 {
    unsafe { RegisterWindowMessageW(windows_sys::w!("LucidPane.DesktopHook.Attach.v1")) }
}

#[must_use]
pub fn geometry_attach_message() -> u32 {
    unsafe { RegisterWindowMessageW(windows_sys::w!("LucidPane.DesktopHook.Geometry.Attach.v1")) }
}

/// Partition actual monitor work areas around panes. Index zero remains uncollected space.
/// No overlap is allowed: Explorer assigns overlapping items to the lowest area index.
/// # Errors
/// Rejects invalid/overlapping panes, missing free space, or too many native work areas.
pub fn partition(monitors: &[Area], panes: &[Area]) -> Result<Vec<Area>, String> {
    if monitors.is_empty()
        || monitors.iter().any(|r| !r.valid())
        || panes.iter().any(|r| !r.valid())
    {
        return Err("桌面或分组工作区域无效".into());
    }
    for (i, pane) in panes.iter().enumerate() {
        if !monitors.iter().any(|m| m.intersect(*pane) == Some(*pane)) {
            return Err("分组需要完整位于一个显示器的工作区内".into());
        }
        if panes[..i].iter().any(|p| p.intersect(*pane).is_some()) {
            return Err("原生分组工作区不能重叠，请移动分组后重试".into());
        }
    }
    let mut remaining = monitors.to_vec();
    for pane in panes {
        let mut next = Vec::new();
        for rect in remaining {
            let Some(cut) = rect.intersect(*pane) else {
                next.push(rect);
                continue;
            };
            for piece in [
                Area {
                    bottom: cut.top,
                    ..rect
                },
                Area {
                    top: cut.bottom,
                    ..rect
                },
                Area {
                    top: cut.top,
                    bottom: cut.bottom,
                    right: cut.left,
                    ..rect
                },
                Area {
                    top: cut.top,
                    bottom: cut.bottom,
                    left: cut.right,
                    ..rect
                },
            ] {
                if piece.valid() {
                    next.push(piece);
                }
            }
        }
        remaining = next;
    }
    if remaining.is_empty() {
        return Err("需要为未收纳桌面图标保留工作区域".into());
    }
    // Use the largest free rectangle for newly appearing/unassigned items.
    remaining.sort_by_key(|r| {
        std::cmp::Reverse(i64::from(r.right - r.left) * i64::from(r.bottom - r.top))
    });
    remaining.extend_from_slice(panes);
    if remaining.len() > MAX_AREAS {
        return Err("原生工作区数量超过 Windows 的 16 个区域限制".into());
    }
    Ok(remaining)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partitions_without_overlap_and_preserves_every_pixel() {
        let desktop = Area {
            left: -100,
            top: 0,
            right: 900,
            bottom: 700,
        };
        let panes = [
            Area {
                left: 100,
                top: 100,
                right: 300,
                bottom: 400,
            },
            Area {
                left: 400,
                top: 300,
                right: 800,
                bottom: 600,
            },
        ];
        let areas = partition(&[desktop], &panes).unwrap();
        for y in 0..700 {
            for x in -100..900 {
                assert_eq!(areas.iter().filter(|r| r.contains(x, y)).count(), 1);
            }
        }
        assert_eq!(&areas[areas.len() - 2..], &panes);
    }
    #[test]
    fn rejects_overlapping_or_outside_panes_without_changing_layout() {
        let desktop = Area {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        };
        assert!(partition(&[desktop], &[desktop, desktop]).is_err());
        assert!(
            partition(
                &[desktop],
                &[Area {
                    right: 101,
                    ..desktop
                }]
            )
            .is_err()
        );
        assert!(partition(&[desktop], &[desktop]).is_err());
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
