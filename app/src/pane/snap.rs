use windows_sys::Win32::Foundation::RECT;

/// Derive each proposal from the original pointer offset, never from a snapped frame.
pub struct DragOrigin {
    bounds: RECT,
    pointer: windows_sys::Win32::Foundation::POINT,
}
impl DragOrigin {
    pub fn new(bounds: RECT, pointer: windows_sys::Win32::Foundation::POINT) -> Self {
        Self { bounds, pointer }
    }
    pub fn proposal(&self, pointer: windows_sys::Win32::Foundation::POINT) -> RECT {
        let dx = pointer.x - self.pointer.x;
        let dy = pointer.y - self.pointer.y;
        RECT {
            left: self.bounds.left + dx,
            right: self.bounds.right + dx,
            top: self.bounds.top + dy,
            bottom: self.bounds.bottom + dy,
        }
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

/// Resize only the dragged edges. Peer alignment overrides grid snapping, but
/// capture distance always comes from the original Windows sizing proposal.
pub fn resize(
    rect: &mut RECT,
    proposal: &RECT,
    edge: u32,
    peers: &[RECT],
    gap: i32,
    threshold: i32,
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let left = matches!(edge, WMSZ_LEFT | WMSZ_TOPLEFT | WMSZ_BOTTOMLEFT);
    let right = matches!(edge, WMSZ_RIGHT | WMSZ_TOPRIGHT | WMSZ_BOTTOMRIGHT);
    let top = matches!(edge, WMSZ_TOP | WMSZ_TOPLEFT | WMSZ_TOPRIGHT);
    let bottom = matches!(edge, WMSZ_BOTTOM | WMSZ_BOTTOMLEFT | WMSZ_BOTTOMRIGHT);
    let nearest = |value: i32, candidates: Vec<i32>| {
        candidates
            .into_iter()
            .filter(|target| (target - value).abs() <= threshold)
            .min_by_key(|target| (target - value).abs())
    };
    let beside = |peer: &&RECT| {
        ((proposal.left - peer.right).abs() <= gap + threshold
            || (proposal.right - peer.left).abs() <= gap + threshold)
            && proposal.top < peer.bottom + threshold
            && proposal.bottom > peer.top - threshold
    };
    let stacked = |peer: &&RECT| {
        ((proposal.top - peer.bottom).abs() <= gap + threshold
            || (proposal.bottom - peer.top).abs() <= gap + threshold)
            && proposal.left < peer.right + threshold
            && proposal.right > peer.left - threshold
    };
    if left || right {
        let value = if left { proposal.left } else { proposal.right };
        let candidates = peers
            .iter()
            .filter(stacked)
            .map(|peer| if left { peer.left } else { peer.right })
            .filter(|target| {
                if left {
                    *target < proposal.right
                } else {
                    *target > proposal.left
                }
            })
            .collect();
        if let Some(target) = nearest(value, candidates) {
            if left {
                rect.left = target;
            } else {
                rect.right = target;
            }
        }
    }
    if top || bottom {
        let value = if top { proposal.top } else { proposal.bottom };
        let candidates = peers
            .iter()
            .filter(beside)
            .map(|peer| if top { peer.top } else { peer.bottom })
            .filter(|target| {
                if top {
                    *target < proposal.bottom
                } else {
                    *target > proposal.top
                }
            })
            .collect();
        if let Some(target) = nearest(value, candidates) {
            if top {
                rect.top = target;
            } else {
                rect.bottom = target;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resizing_adjacent_panes_aligns_only_dragged_edges() {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        for scale in [1, 2] {
            let peer = RECT {
                left: -400 * scale,
                top: 100 * scale,
                right: 0,
                bottom: 400 * scale,
            };
            for edge in [WMSZ_TOP, WMSZ_BOTTOM, WMSZ_TOPRIGHT, WMSZ_BOTTOMRIGHT] {
                let proposal = RECT {
                    left: 5,
                    top: 106 * scale,
                    right: 305 * scale,
                    bottom: 406 * scale,
                };
                let mut rect = proposal;
                resize(&mut rect, &proposal, edge, &[peer], 5, 14 * scale);
                assert_eq!((rect.left, rect.right), (proposal.left, proposal.right));
                if matches!(edge, WMSZ_TOP | WMSZ_TOPRIGHT) {
                    assert_eq!((rect.top, rect.bottom), (peer.top, proposal.bottom));
                } else {
                    assert_eq!((rect.top, rect.bottom), (proposal.top, peer.bottom));
                }
            }
            for edge in [WMSZ_LEFT, WMSZ_RIGHT, WMSZ_BOTTOMLEFT, WMSZ_BOTTOMRIGHT] {
                let proposal = RECT {
                    left: -394 * scale,
                    top: peer.bottom + 5,
                    right: 6 * scale,
                    bottom: 700 * scale,
                };
                let mut rect = proposal;
                resize(&mut rect, &proposal, edge, &[peer], 5, 14 * scale);
                assert_eq!((rect.top, rect.bottom), (proposal.top, proposal.bottom));
                if matches!(edge, WMSZ_LEFT | WMSZ_BOTTOMLEFT) {
                    assert_eq!((rect.left, rect.right), (peer.left, proposal.right));
                } else {
                    assert_eq!((rect.left, rect.right), (proposal.left, peer.right));
                }
            }
        }
    }

    #[test]
    fn resize_uses_raw_proposal_for_capture_and_releases_beyond_threshold() {
        use windows_sys::Win32::UI::WindowsAndMessaging::WMSZ_BOTTOM;
        let peer = RECT {
            left: 0,
            top: 0,
            right: 300,
            bottom: 300,
        };
        for (bottom, expected) in [(309, 300), (315, 310)] {
            let proposal = RECT {
                left: 305,
                top: 0,
                right: 605,
                bottom,
            };
            let mut grid_snapped = RECT {
                bottom: 310,
                ..proposal
            };
            resize(&mut grid_snapped, &proposal, WMSZ_BOTTOM, &[peer], 5, 14);
            assert_eq!(grid_snapped.bottom, expected);
        }
        let proposal = RECT {
            left: 900,
            top: 0,
            right: 1200,
            bottom: 309,
        };
        let mut rect = proposal;
        resize(&mut rect, &proposal, WMSZ_BOTTOM, &[peer], 5, 14);
        assert_eq!(rect.bottom, 309, "distant panels must not attract resizing");
        let close = RECT {
            bottom: 305,
            ..peer
        };
        let proposal = RECT {
            left: 305,
            top: 0,
            right: 605,
            bottom: 307,
        };
        let mut rect = proposal;
        resize(&mut rect, &proposal, WMSZ_BOTTOM, &[peer, close], 5, 14);
        assert_eq!(rect.bottom, 305, "nearest edge wins");
    }

    #[test]
    fn slow_drag_can_escape_the_top_edge_without_accumulating_snap_feedback() {
        use windows_sys::Win32::Foundation::POINT;
        let origin = DragOrigin::new(
            RECT {
                left: 100,
                top: 12,
                right: 400,
                bottom: 312,
            },
            POINT { x: 150, y: 24 },
        );
        let work = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
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
        let work = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };
        for (left, top, expected) in [(-1915, 5, (-1908, 12)), (-310, 735, (-312, 728))] {
            let mut rect = RECT {
                left,
                top,
                right: left + 300,
                bottom: top + 300,
            };
            snap(&mut rect, &[], Some(&work), 12, 14);
            assert_eq!((rect.left, rect.top), expected);
            assert_eq!((rect.right - rect.left, rect.bottom - rect.top), (300, 300));
        }
        let mut far = RECT {
            left: -1000,
            top: 400,
            right: -700,
            bottom: 700,
        };
        snap(&mut far, &[], Some(&work), 12, 14);
        assert_eq!((far.left, far.top), (-1000, 400));
    }

    #[test]
    fn nearest_pane_edge_wins_over_desktop_edge() {
        let work = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let peer = RECT {
            left: 20,
            top: 0,
            right: 320,
            bottom: 100,
        };
        let mut rect = RECT {
            left: 22,
            top: 114,
            right: 322,
            bottom: 414,
        };
        snap(&mut rect, &[peer], Some(&work), 12, 14);
        assert_eq!((rect.left, rect.top), (20, 112));
    }
}
