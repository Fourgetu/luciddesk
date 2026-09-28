//! Physical window bounds are remembered independently for each display topology.
use super::*;
use desktop_window::MonitorDescriptor;
use std::time::{Duration, Instant};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Initial bounds in DIP, using the same grid metrics as desktop rendering.
pub(super) fn first_pane(monitors: &[MonitorDescriptor], grid_scale: f32) -> RectDip {
    let grid = layout::desktop_grid(0.0, 0.0, 48.0, grid_scale);
    let width = (layout::PADDING * 2.0 + 3.0 * grid.cell_width).max(RectDip::MIN_WIDTH);
    let height = (layout::HEADER + layout::PADDING * 2.0 + 4.0 * grid.cell_height).max(RectDip::MIN_HEIGHT);
    let Some(monitor) = monitors.iter().find(|m| m.primary).or_else(|| monitors.first()) else {
        return RectDip::new(24.0, 24.0, width, height);
    };
    let scale = monitor.dpi as f32 / 96.0;
    let work = monitor.work_area;
    let width = (width * scale).ceil().min(work.width as f32);
    let height = (height * scale).ceil().min(work.height as f32);
    let margin = (24.0 * scale).ceil();
    RectDip {
        x: (work.x as f32 + (work.width as f32 - width - margin).max(0.0)) / scale,
        y: (work.y as f32 + margin.min((work.height as f32 - height).max(0.0))) / scale,
        width: width / scale,
        height: height / scale,
    }
}

pub(super) struct Layouts {
    monitors: Vec<MonitorDescriptor>,
    pending: Option<(Vec<MonitorDescriptor>, Instant)>,
    next_check: Option<Instant>,
    pub positions: HashMap<PanelId, RectDip>,
}
impl Default for Layouts {
    fn default() -> Self {
        Self {
            monitors: Vec::new(),
            pending: None,
            next_check: Some(Instant::now()),
            positions: HashMap::new(),
        }
    }
}
impl Layouts {
    pub(super) fn invalidate(&mut self) {
        self.next_check = Some(Instant::now());
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.next_check
    }
}
fn key(monitors: &[MonitorDescriptor]) -> String {
    let mut parts: Vec<_> = monitors
        .iter()
        .map(|m| format!("{}:{:?}:{}", m.id.as_str(), m.work_area, m.dpi))
        .collect();
    parts.sort();
    parts.join("|")
}

fn fit(mut r: RectDip, monitors: &[MonitorDescriptor]) -> RectDip {
    let distance = |m: &&MonitorDescriptor| {
        let w = m.work_area;
        let x = r.x + r.width / 2.0;
        let y = r.y + r.height / 2.0;
        let dx = x - x.clamp(w.x as f32, (w.x + w.width) as f32);
        let dy = y - y.clamp(w.y as f32, (w.y + w.height) as f32);
        (dx * dx + dy * dy) as u64
    };
    if let Some(m) = monitors.iter().min_by_key(distance) {
        let w = m.work_area;
        r.width = r.width.min(w.width as f32).max(1.0);
        r.height = r.height.min(w.height as f32).max(1.0);
        r.x = r.x.clamp(w.x as f32, (w.x + w.width) as f32 - r.width);
        r.y = r.y.clamp(w.y as f32, (w.y + w.height) as f32 - r.height);
    }
    r
}

fn monitor_scale(r: RectDip, monitors: &[MonitorDescriptor]) -> f32 {
    monitors
        .iter()
        .find(|m| {
            let b = m.bounds;
            r.x >= b.x as f32
                && r.x < (b.x + b.width) as f32
                && r.y >= b.y as f32
                && r.y < (b.y + b.height) as f32
        })
        .or_else(|| monitors.first())
        .map_or(1.0, |m| m.dpi as f32 / 96.0)
}

pub(super) fn initialize(s: &mut PaneApp, monitors: Vec<MonitorDescriptor>) -> Result<(), String> {
    if monitors.is_empty() {
        return Ok(());
    }
    let saved: HashMap<_, _> = s
        .store
        .monitor_layout(&key(&monitors))
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    let Some(runtime) = &mut s.runtime else {
        return Ok(());
    };
    let primary_scale = monitors.first().map_or(1.0, |m| m.dpi as f32 / 96.0);
    let mut positions = HashMap::new();
    let ids: Vec<_> = s.workspace.panels().iter().map(Panel::id).collect();
    for id in ids {
        let panel = s.workspace.panel_mut(id).unwrap();
        let old = panel.rect();
        let physical = saved
            .get(&id)
            .or_else(|| runtime.layouts.positions.get(&id))
            .copied()
            .unwrap_or(RectDip::new(
                old.x * primary_scale,
                old.y * primary_scale,
                old.width * primary_scale,
                old.height * primary_scale,
            ));
        let physical = fit(physical, &monitors);
        let scale = monitor_scale(physical, &monitors);
        panel.set_rect(RectDip::new(
            physical.x / scale,
            physical.y / scale,
            physical.width / scale,
            physical.height / scale,
        ));
        positions.insert(id, physical);
    }
    for group in s.workspace.tab_groups() {
        if let Some(position) = positions.get(&group.active).copied() {
            for id in &group.members { positions.insert(*id, position); }
        }
    }
    s.workspace.sync_tab_windows();
    runtime.layouts = Layouts {
        monitors,
        pending: None,
        next_check: None,
        positions,
    };
    Ok(())
}

pub(super) fn place(s: &PaneApp, id: PanelId) {
    let Some(runtime) = &s.runtime else {
        return;
    };
    let Some(r) = runtime.layouts.positions.get(&id) else {
        return;
    };
    let Some(v) = s.views.iter().find(|v| v.id == id) else {
        return;
    };
    let scale = monitor_scale(*r, &runtime.layouts.monitors);
    let panel = s.workspace.panel(id).unwrap();
    let height = if panel.is_search() {
        56.0 * scale
    } else if panel.collapsed() {
        layout::HEADER * scale
    } else {
        r.height
    };
    unsafe {
        SetWindowPos(
            v.window.hwnd().cast(),
            std::ptr::null_mut(),
            r.x as i32,
            r.y as i32,
            r.width as i32,
            height as i32,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
        if panel.is_search() {
            PostMessageW(v.window.hwnd().cast(), search::RESTORE_LAYOUT, 0, 0);
        }
    }
}

pub(super) fn record(s: &mut PaneApp) -> Result<(), String> {
    let Some(runtime) = &mut s.runtime else {
        return Ok(());
    };
    let current = desktop_window::enumerate_monitors();
    if runtime.layouts.monitors != current || runtime.layouts.pending.is_some() {
        return Ok(());
    }
    for view in &s.views {
        let mut r = RECT::default();
        if unsafe { GetWindowRect(view.window.hwnd().cast(), &raw mut r) } == 0 {
            continue;
        }
        let scale = unsafe { GetDpiForWindow(view.window.hwnd().cast()) }.max(96) as f32 / 96.0;
        let panel = s.workspace.panel(view.id).unwrap();
        runtime.layouts.positions.insert(
            view.id,
            RectDip::new(
                r.left as f32,
                r.top as f32,
                (r.right - r.left) as f32,
                if panel.collapsed() || panel.is_search() {
                    panel.rect().height * scale
                } else {
                    (r.bottom - r.top) as f32
                },
            ),
        );
    }
    for group in s.workspace.tab_groups() {
        if let Some(position) = runtime.layouts.positions.get(&group.active).copied() {
            for id in &group.members { runtime.layouts.positions.insert(*id, position); }
        }
    }
    runtime
        .layouts
        .positions
        .retain(|id, _| s.workspace.panel(*id).is_some());
    let layout: Vec<_> = runtime
        .layouts
        .positions
        .iter()
        .map(|(id, r)| (*id, *r))
        .collect();
    s.store
        .save_monitor_layout(&key(&current), &layout)
        .map_err(|e| e.to_string())
}

pub(super) fn tick(state: &Rc<RefCell<PaneApp>>) -> Result<(), String> {
    {
        let mut s = state.borrow_mut();
        let Some(runtime) = &mut s.runtime else { return Ok(()); };
        if runtime.layouts.next_check.is_none_or(|due| due > Instant::now()) {
            return Ok(());
        }
        // Retry transient empty topology and wait for a stable display arrangement.
        runtime.layouts.next_check = Some(Instant::now() + Duration::from_secs(2));
    }
    let current = desktop_window::enumerate_monitors();
    if current.is_empty() {
        return Ok(());
    }
    {
        let mut s = state.borrow_mut();
        let Some(runtime) = &mut s.runtime else {
            return Ok(());
        };
        if runtime.layouts.monitors == current {
            runtime.layouts.pending = None;
            runtime.layouts.next_check = None;
            return Ok(());
        }
        match &runtime.layouts.pending {
            Some((pending, since))
                if *pending == current && since.elapsed() >= Duration::from_secs(2) => {}
            Some((pending, _)) if *pending == current => return Ok(()),
            _ => {
                runtime.layouts.pending = Some((current, Instant::now()));
                return Ok(());
            }
        }
        initialize(&mut s, current)?;
    }
    let ids: Vec<_> = state.borrow().views.iter().map(|v| v.id).collect();
    for id in ids {
        place(&state.borrow(), id);
    }
    record(&mut state.borrow_mut())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor(dpi: u32, width: i32, height: i32) -> MonitorDescriptor {
        let work_area = desktop_window::PixelRect { x: 80, y: 40, width, height };
        MonitorDescriptor {
            id: desktop_core::MonitorId::new("primary"),
            bounds: work_area,
            work_area,
            dpi,
            primary: true,
        }
    }

    #[test]
    fn first_pane_fits_three_columns_and_four_rows_at_fractional_dpi() {
        for dpi in [96, 120, 144, 168, 192] {
            for grid_scale in [100.0, 125.0, 150.0] {
                let m = monitor(dpi, 2400, 1800);
                let r = first_pane(std::slice::from_ref(&m), grid_scale);
                let scale = dpi as f32 / 96.0;
                let grid = layout::desktop_grid(r.width, r.height, 48.0, grid_scale);
                assert_eq!((grid.columns, grid.visible_rows), (3, 4));
                assert_eq!(grid.max_scroll(12), 0);
                assert!((r.x * scale + r.width * scale + (24.0 * scale).ceil() - 2480.0).abs() < 0.01);
                assert!((r.y * scale - 40.0 - (24.0 * scale).ceil()).abs() < 0.01);
            }
        }
        let r = first_pane(&[monitor(96, 1920, 1040)], 100.0);
        assert_eq!((r.width, r.height), (288.0, 448.0));
    }

    #[test]
    fn first_pane_uses_primary_and_stays_inside_small_work_area() {
        let primary = monitor(144, 240, 300);
        let mut secondary = monitor(96, 1920, 1080);
        secondary.primary = false;
        let r = first_pane(&[secondary, primary], 100.0);
        assert_eq!(r, RectDip { x: 80.0 / 1.5, y: 40.0 / 1.5, width: 160.0, height: 200.0 });
        assert_eq!(first_pane(&[], 100.0), RectDip::new(24.0, 24.0, 288.0, 448.0));
    }

    #[test]
    fn settled_layout_is_not_enumerated_until_invalidated() {
        let mut app = super::super::tests::test_state();
        app.runtime = Some(runtime::State::new(std::path::PathBuf::from("unused.db")));
        app.runtime.as_mut().unwrap().layouts.next_check = None;
        let state = Rc::new(RefCell::new(app));
        tick(&state).unwrap();
        assert!(state.borrow().runtime.as_ref().unwrap().layouts.pending.is_none());
        state.borrow_mut().runtime.as_mut().unwrap().layouts.invalidate();
        tick(&state).unwrap();
        let s = state.borrow();
        assert!(s.runtime.as_ref().unwrap().layouts.deadline().is_some());
    }
    #[test]
    fn removed_monitor_moves_panes_into_work_area_without_minimum_width() {
        let monitor = MonitorDescriptor {
            id: desktop_core::MonitorId::new("primary"),
            bounds: desktop_window::PixelRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            work_area: desktop_window::PixelRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            dpi: 96,
            primary: true,
        };
        let r = fit(RectDip::new(-1200.0, -900.0, 80.0, 300.0), &[monitor]);
        assert_eq!(r, RectDip::new(0.0, 0.0, 80.0, 300.0));
    }
}
