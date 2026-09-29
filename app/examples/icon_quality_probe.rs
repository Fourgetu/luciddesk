//! cargo run -p luciddesk --example icon_quality_probe -- <shortcut> <output.bmp>
//! Read-only Shell extraction and a 72-pixel preview for inspecting real icon edges.
#[path = "../src/diagnostics.rs"]
#[allow(dead_code)]
mod diagnostics;
#[path = "../src/i18n.rs"]
#[allow(dead_code)]
mod i18n;
#[path = "../src/pane/assets.rs"]
#[allow(dead_code)]
mod assets;
#[path = "../src/pane/fonts.rs"]
#[allow(dead_code)]
mod fonts;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _apartment = desktop_shell::ShellApartment::initialize_sta()?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected shortcut path and output BMP path".into());
    }
    let path = std::path::PathBuf::from(&args[0]);
    let identity = desktop_core::ShellIdentity::FileSystem {
        path,
        volume_id: None,
        file_id: None,
    };
    let source = assets::load(&identity, 128)?;
    let pixels = assets::resample(&source, 72, 72)?;
    println!("source={}x{}, preview=72x72", source.width, source.height);
    let mut data = pixels.into_owned().data;
    for pixel in data.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = (u16::from(*channel) + (255 - alpha) * 24 / 255).min(255) as u8;
        }
        pixel[3] = 255;
    }
    let mut bmp = vec![0u8; 54];
    bmp[..2].copy_from_slice(b"BM");
    bmp[2..6].copy_from_slice(&(54 + data.len() as u32).to_le_bytes());
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&72i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&(-72i32).to_le_bytes());
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&32u16.to_le_bytes());
    bmp.extend(data);
    std::fs::write(&args[1], bmp)?;
    Ok(())
}
