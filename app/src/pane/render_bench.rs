//! Interactive, repeatable measurements; no timing thresholds in CI.
use super::{composition::Surface, menu::Entry, render::Renderer};
use std::time::Instant;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[test]
#[ignore = "Shows four GPU windows; run alone on an interactive desktop"]
fn multi_window_render_latency() {
    let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
    let start = Instant::now();
    let mut windows = Vec::new();
    let mut surfaces = Vec::new();
    let renderer = Renderer::new().unwrap();
    let rows: Vec<_> = (1..=10)
        .map(|id| Entry {
            id,
            label: "文件夹面板与菜单",
            icon: "",
            trailing: "Ctrl+L",
        })
        .collect();
    for i in 0..4 {
        let window = windows_window::Window::new("LucidPane GPU benchmark")
            .size(320, 340)
            .style(WS_POPUP)
            .ex_style(WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP)
            .on_message(|_, msg, _, _| matches!(msg, WM_DESTROY | WM_ERASEBKGND).then_some(0))
            .create()
            .unwrap();
        unsafe {
            SetWindowPos(
                window.hwnd().cast(),
                std::ptr::null_mut(),
                40 + i * 330,
                80,
                320,
                340,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
        let mut surface =
            Surface::new(windows::Win32::Foundation::HWND(window.hwnd().cast())).unwrap();
        surface.material(
            windows::Win32::Foundation::HWND(window.hwnd().cast()),
            desktop_core::Backdrop::Acrylic,
        );
        surfaces.push(surface);
        windows.push(window);
    }
    let cold = start.elapsed();
    let mut draw = Vec::new();
    let mut present = Vec::new();
    for round in 0..32 {
        for surface in &mut surfaces {
            let start = Instant::now();
            let target = surface.begin_frame(320, 340).unwrap();
            renderer
                .paint_flyout(
                    &target,
                    320,
                    340,
                    1.0,
                    &rows,
                    &(0..rows.len())
                        .map(|i| if i == round % 10 { 1.0 } else { 0.0 })
                        .collect::<Vec<_>>(),
                    surface.native,
                    true,
                )
                .unwrap();
            draw.push(start.elapsed().as_micros());
            let start = Instant::now();
            surface.end_frame().unwrap();
            present.push(start.elapsed().as_micros());
        }
    }
    draw.sort_unstable();
    present.sort_unstable();
    println!(
        "cold_four_surfaces_ms={} draw_p50_us={} draw_p95_us={} present_p50_us={} present_p95_us={} present_total_us={}",
        cold.as_millis(),
        draw[64],
        draw[121],
        present[64],
        present[121],
        present.iter().sum::<u128>()
    );
    drop(surfaces);
    drop(windows);
}
