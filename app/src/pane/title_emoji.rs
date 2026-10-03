//! Global title emoji rendering preference, included in workspace backups.
use std::sync::atomic::{AtomicBool, Ordering};

static COLOR: AtomicBool = AtomicBool::new(true);
const KEY: &str = "pane_title_emoji_color";

pub(super) fn color() -> bool {
    COLOR.load(Ordering::Relaxed)
}

pub(super) fn load(store: &luciddesk_storage::WorkspaceStore) -> Result<(), String> {
    let value = store.preference(KEY).map_err(|e| e.to_string())?;
    COLOR.store(value.as_deref() != Some("false"), Ordering::Relaxed);
    Ok(())
}

pub(super) fn save(store: &luciddesk_storage::WorkspaceStore, color: bool) -> Result<(), String> {
    store
        .save_preference(KEY, if color { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    COLOR.store(color, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_emoji_mode_persists_and_changes_rendered_pixels() {
        use crate::pane::{canvas, native_graphics::canvas_result};
        use windows_canvas::{ColorF, GpuDevice, ParagraphAlignment, TextFormat, TextLayout};
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                COLOR.store(self.0, Ordering::Relaxed);
            }
        }
        let _restore = Restore(color());
        let store = luciddesk_storage::WorkspaceStore::open_in_memory().unwrap();
        load(&store).unwrap();
        assert!(color());
        let device = GpuDevice::new_warp().unwrap();
        let format = TextFormat::new("Segoe UI", 16.0)
            .unwrap()
            .with_paragraph_alignment(ParagraphAlignment::Center);
        canvas::apply_fallback(&format).unwrap();
        let layout = TextLayout::new("📁 AI", &format, 140.0, 40.0).unwrap();
        let surface = canvas::Offscreen::new(&device, 140, 40).unwrap();
        for enabled in [true, false] {
            save(&store, enabled).unwrap();
            COLOR.store(!enabled, Ordering::Relaxed);
            load(&store).unwrap();
            assert_eq!(color(), enabled);
            canvas::draw(&surface.target, 1.0, |frame| {
                frame.clear(ColorF::default());
                let brush = canvas_result(frame.create_solid_brush(ColorF::WHITE))?;
                frame.clipped_color_layout("📁 AI", &layout, 0.0, 0.0, &brush)?;
                frame.finish()
            })
            .unwrap();
            let pixels = surface.pixels().unwrap();
            assert!(pixels.chunks_exact(4).any(|p| p[3] > 100));
            assert_eq!(
                pixels.chunks_exact(4).any(|p| p[0] != p[1] || p[1] != p[2]),
                enabled
            );
        }
        assert_eq!(
            store
                .backup_snapshot()
                .unwrap()
                .preference(KEY)
                .unwrap()
                .as_deref(),
            Some("false")
        );
    }
}
