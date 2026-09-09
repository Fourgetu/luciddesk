use windows_sys::Win32::Foundation::RECT;

/// Derive each proposal from the original pointer offset, never from a snapped frame.
pub struct DragOrigin {
    bounds: RECT,
    pointer: windows_sys::Win32::Foundation::POINT,
}
impl DragOrigin {
    pub fn new(bounds: RECT, pointer: windows_sys::Win32::Foundation::POINT) -> Self { Self { bounds, pointer } }
    pub fn proposal(&self, pointer: windows_sys::Win32::Foundation::POINT) -> RECT {
        let dx = pointer.x - self.pointer.x;
        let dy = pointer.y - self.pointer.y;
        RECT { left: self.bounds.left + dx, right: self.bounds.right + dx,
            top: self.bounds.top + dy, bottom: self.bounds.bottom + dy }
    }
}

// Choose the nearest edge independently on each axis, using the actual visible pane bounds.
pub fn snap(rect: &mut RECT, peers: &[RECT], work: Option<&RECT>, gap: i32, threshold: i32) {
    let nearest = |values: Vec<i32>| {
        values
            .into_iter()
            .filter(|delta| delta.abs() <= threshold)
            .min_by_key(|delta| delta.abs())
            .unwrap_or(0)
    };
    let mut horizontal = Vec::new();
    if let Some(work) = work {
        horizontal.extend([work.left + gap - rect.left, work.right - gap - rect.right]);
    }
    for peer in peers {
        if rect.top < peer.bottom + gap + threshold && rect.bottom > peer.top - gap - threshold {
            horizontal.extend([peer.right + gap - rect.left, peer.left - gap - rect.right]);
            if (rect.top - peer.bottom).abs() <= gap + threshold
                || (rect.bottom - peer.top).abs() <= gap + threshold
            {
                horizontal.extend([peer.left - rect.left, peer.right - rect.right]);
            }
        }
    }
    let delta = nearest(horizontal);
    rect.left += delta;
    rect.right += delta;
    let mut vertical = Vec::new();
    if let Some(work) = work {
        vertical.extend([work.top + gap - rect.top, work.bottom - gap - rect.bottom]);
    }
    for peer in peers {
        if rect.left < peer.right + gap + threshold && rect.right > peer.left - gap - threshold {
            vertical.extend([peer.bottom + gap - rect.top, peer.top - gap - rect.bottom]);
            if (rect.left - peer.right).abs() <= gap + threshold
                || (rect.right - peer.left).abs() <= gap + threshold
            {
                vertical.extend([peer.top - rect.top, peer.bottom - rect.bottom]);
            }
        }
    }
    let delta = nearest(vertical);
    rect.top += delta;
    rect.bottom += delta;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_drag_can_escape_the_top_edge_without_accumulating_snap_feedback() {
        use windows_sys::Win32::Foundation::POINT;
        let origin = DragOrigin::new(RECT { left: 100, top: 12, right: 400, bottom: 312 }, POINT { x: 150, y: 24 });
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        let mut result = RECT::default();
        for dy in 1..=100 {
            result = origin.proposal(POINT { x: 150, y: 24 + dy });
            snap(&mut result, &[], Some(&work), 12, 14);
        }
        assert_eq!((result.left, result.top, result.bottom), (100, 112, 412));
    }
    #[test]
    fn adjacent_panes_keep_gap_align_and_preserve_size() {
        let peer = RECT {
            left: -400,
            top: 100,
            right: 0,
            bottom: 400,
        };
        let mut rect = RECT {
            left: 18,
            top: 106,
            right: 318,
            bottom: 406,
        };
        snap(&mut rect, &[peer], None, 12, 14);
        assert_eq!(
            (rect.left, rect.top, rect.right, rect.bottom),
            (12, 100, 312, 400)
        );
        let mut far = RECT {
            left: 500,
            top: 600,
            right: 800,
            bottom: 900,
        };
        snap(&mut far, &[peer], None, 12, 14);
        assert_eq!((far.left, far.top), (500, 600));
    }

    #[test]
    fn desktop_corners_respect_work_area_and_negative_monitor_coordinates() {
        let work = RECT { left: -1920, top: 0, right: 0, bottom: 1040 };
        for (left, top, expected) in [(-1915, 5, (-1908, 12)), (-310, 735, (-312, 728))] {
            let mut rect = RECT { left, top, right: left + 300, bottom: top + 300 };
            snap(&mut rect, &[], Some(&work), 12, 14);
            assert_eq!((rect.left, rect.top), expected);
            assert_eq!((rect.right - rect.left, rect.bottom - rect.top), (300, 300));
        }
        let mut far = RECT { left: -1000, top: 400, right: -700, bottom: 700 };
        snap(&mut far, &[], Some(&work), 12, 14);
        assert_eq!((far.left, far.top), (-1000, 400));
    }

    #[test]
    fn nearest_pane_edge_wins_over_desktop_edge() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        let peer = RECT { left: 20, top: 0, right: 320, bottom: 100 };
        let mut rect = RECT { left: 22, top: 114, right: 322, bottom: 414 };
        snap(&mut rect, &[peer], Some(&work), 12, 14);
        assert_eq!((rect.left, rect.top), (20, 112));
    }
}
