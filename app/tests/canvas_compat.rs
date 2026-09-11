//! Executable compatibility probes for the production Canvas dependency.
use canvas_core::Interface as CanvasInterface;
use windows::Win32::Graphics::Direct2D::{D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, ID2D1DeviceContext};
use windows::Win32::Graphics::Dxgi::Common::DXGI_ALPHA_MODE_PREMULTIPLIED;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain1;
use windows::core::Interface;
use windows_canvas::*;

// The two crates generate the same COM ABI with different Rust types. Clone the
// borrowed interface (AddRef); never take ownership of Canvas's reference.
fn native(session: &DrawingSession<'_>) -> ID2D1DeviceContext {
    unsafe {
        ID2D1DeviceContext::from_raw_borrowed(&session.raw().as_raw())
            .unwrap()
            .clone()
    }
}

fn pixel(bytes: &[u8], width: usize, x: usize, y: usize) -> &[u8] {
    &bytes[(y * width + x) * 4..(y * width + x + 1) * 4]
}

#[test]
fn transparent_bgra_icons_survive_cached_redraw() -> Result<()> {
    let device = GpuDevice::new_warp()?;
    let target = device.create_render_target(32, 32)?;
    let mut cached = None;
    target.draw(|session| {
        // Same tightly packed premultiplied BGRA contract as the Shell icon cache.
        cached = Some(session.create_bitmap(&[32, 64, 128, 128].repeat(16), 4, 4)?);
        Ok(())
    })?;
    for _ in 0..3 {
        target.draw(|session| {
            session.clear(ColorF::default());
            session.draw_bitmap(
                cached.as_ref().unwrap(),
                &Rect::from_xywh(8.0, 8.0, 4.0, 4.0),
                1.0,
            );
            Ok(())
        })?;
        let pixels = target.read_pixels()?;
        assert_eq!(pixel(&pixels, 32, 0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(&pixels, 32, 9, 9), [32, 64, 128, 128]);
    }
    Ok(())
}

#[test]
fn dip_geometry_scales_at_96_120_144_and_192_dpi() -> Result<()> {
    let device = GpuDevice::new_warp()?;
    for (dpi, edge) in [(96.0, 16), (120.0, 20), (144.0, 24), (192.0, 32)] {
        let target = device.create_render_target(64, 64)?;
        target.draw(|session| {
            unsafe { native(session).SetDpi(dpi, dpi) };
            session.clear(ColorF::default());
            let brush = session.create_solid_brush(ColorF::rgb(1.0, 1.0, 1.0))?;
            session.fill_rect(&Rect::new(0.0, 0.0, 16.0, 16.0), &brush);
            Ok(())
        })?;
        let pixels = target.read_pixels()?;
        assert_eq!(pixel(&pixels, 64, edge - 1, edge - 1)[3], 255);
        assert_eq!(pixel(&pixels, 64, edge, edge)[3], 0);
    }
    Ok(())
}

#[test]
fn chinese_labels_have_grayscale_alpha_and_wrap() -> Result<()> {
    let device = GpuDevice::new_warp()?;
    let target = device.create_render_target(240, 120)?;
    let format = TextFormat::new("Microsoft YaHei UI", 16.0)?
        .with_alignment(TextAlignment::Center)
        .with_word_wrapping(WordWrapping::Wrap);
    let layout = TextLayout::new("微信 网易云音乐 桌面分组名称", &format, 80.0, 120.0)?;
    assert!(layout.metrics().line_count > 1);
    assert!(layout.metrics().height > 16.0);
    target.draw(|session| {
        unsafe { native(session).SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
        session.clear(ColorF::default());
        let white = session.create_solid_brush(ColorF::rgb(1.0, 1.0, 1.0))?;
        session.draw_text_layout(Vector2 { x: 0.0, y: 0.0 }, &layout, &white);
        Ok(())
    })?;
    let pixels = target.read_pixels()?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[3] > 0 && p[3] < 255)
    );
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] == p[1] && p[1] == p[2] && p[2] <= p[3])
    );
    assert_eq!(pixel(&pixels, 240, 239, 119), [0, 0, 0, 0]);
    Ok(())
}

#[test]
fn composition_swap_chain_preserves_alpha_and_dpi_after_resize() -> Result<()> {
    let device = GpuDevice::new_warp()?;
    let mut swap = device.create_swap_chain(64, 64)?;
    swap.set_dpi(144.0, 144.0);
    for (width, height) in [(128, 96), (240, 160), (64, 64)] {
        swap.resize(width, height)?;
        // Verify compatibility with the project's existing DirectComposition API.
        let raw = swap.raw_swap_chain().as_raw();
        let native_swap = unsafe { IDXGISwapChain1::from_raw_borrowed(&raw).unwrap() };
        let desc = unsafe { native_swap.GetDesc1().unwrap() };
        assert_eq!((desc.Width, desc.Height), (width, height));
        assert_eq!(desc.AlphaMode, DXGI_ALPHA_MODE_PREMULTIPLIED);
        {
            let session = swap.begin_draw()?;
            let (mut x, mut y) = (0.0, 0.0);
            unsafe { native(&session).GetDpi(&raw mut x, &raw mut y) };
            assert_eq!((x, y), (144.0, 144.0));
            session.clear(ColorF::default());
        }
        assert!(!swap.is_device_lost());
    }
    Ok(())
}
