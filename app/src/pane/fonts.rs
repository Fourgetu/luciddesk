//! Installed outline fonts with basic Chinese/Latin coverage, shared by GDI and DirectWrite.
use std::sync::RwLock;
use windows_sys::Win32::Graphics::Gdi::*;
const KEY: &str = "ui_font_family";
static FAMILY: RwLock<String> = RwLock::new(String::new());

const ICON_GLYPHS: &[u32] = &[
    0xe70d, 0xe70e, 0xe711, 0xe721, 0xe72b, 0xe73e, 0xe76c,
    0xe790, 0xe80f, 0xe81c, 0xe890, 0xe8a5, 0xe8b7, 0xe8bb,
    0xe8d2, 0xe916, 0xe921, 0xe922, 0xe923, 0xe946, 0xf0e2,
];

/// Private-use glyphs cannot rely on normal text fallback. Check the actual
/// DirectWrite family and glyph coverage before selecting the Windows 11 font.
pub(super) fn icon_family() -> &'static str {
    static ICON_FAMILY: std::sync::LazyLock<&'static str> = std::sync::LazyLock::new(|| {
        let family = choose_icon_family(|name| icon_coverage(name).unwrap_or(false));
        crate::diagnostics::render_trace(format_args!("icon_font={family}"));
        family
    });
    *ICON_FAMILY
}

fn choose_icon_family(supports: impl FnOnce(&str) -> bool) -> &'static str {
    if supports("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" }
}

fn icon_coverage(name: &str) -> windows::core::Result<bool> {
    use windows::Win32::Graphics::DirectWrite::*;
    unsafe {
        let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let mut collection = None;
        factory.GetSystemFontCollection(&raw mut collection, false)?;
        let collection = collection.unwrap();
        let mut index = 0;
        let mut exists = windows::core::BOOL(0);
        collection.FindFamilyName(&windows::core::HSTRING::from(name), &raw mut index, &raw mut exists)?;
        if !exists.as_bool() { return Ok(false); }
        let font = collection.GetFontFamily(index)?.GetFirstMatchingFont(
            DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
        )?;
        let face = font.CreateFontFace()?;
        let mut glyphs = vec![0u16; ICON_GLYPHS.len()];
        face.GetGlyphIndices(ICON_GLYPHS.as_ptr(), ICON_GLYPHS.len() as u32, glyphs.as_mut_ptr())?;
        Ok(glyphs.iter().all(|glyph| *glyph != 0))
    }
}
pub(super) fn family() -> String {
    let value = FAMILY.read().unwrap();
    if value.is_empty() {
        super::assets::UI_FONT.into()
    } else {
        value.clone()
    }
}
fn set(value: String) {
    *FAMILY.write().unwrap() = value;
}

unsafe extern "system" fn collect(
    font: *const LOGFONTW,
    _: *const TEXTMETRICW,
    kind: u32,
    data: isize,
) -> i32 {
    if kind & TRUETYPE_FONTTYPE != 0 {
        let font = unsafe { &*font };
        let len = font
            .lfFaceName
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(font.lfFaceName.len());
        let name = String::from_utf16_lossy(&font.lfFaceName[..len]);
        if regular_face(font)
            && !name.starts_with('@')
            && !name.is_empty()
            && font.lfCharSet != SYMBOL_CHARSET
        {
            unsafe { &mut *(data as *mut Vec<String>) }.push(name);
        }
    }
    1
}
fn regular_face(font: &LOGFONTW) -> bool {
    font.lfWeight == FW_NORMAL as i32 && font.lfItalic == 0
}
fn readable(name: &str) -> bool {
    if name.encode_utf16().count() >= 32 || name.contains('\0') {
        return false;
    }
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return false;
        }
        let mut lf = LOGFONTW {
            lfHeight: -16,
            lfWeight: FW_NORMAL as i32,
            ..Default::default()
        };
        for (out, unit) in lf.lfFaceName.iter_mut().zip(name.encode_utf16()) {
            *out = unit;
        }
        let font = CreateFontIndirectW(&lf);
        if font.is_null() {
            DeleteDC(dc);
            return false;
        }
        let old = SelectObject(dc, font);
        let probe: Vec<u16> = "中文标签文件设置AaZz0123456789".encode_utf16().collect();
        let mut glyphs = vec![0; probe.len()];
        let count = GetGlyphIndicesW(
            dc,
            probe.as_ptr(),
            probe.len() as i32,
            glyphs.as_mut_ptr(),
            GGI_MARK_NONEXISTING_GLYPHS,
        );
        SelectObject(dc, old);
        DeleteObject(font);
        DeleteDC(dc);
        count != u32::MAX && glyphs.iter().all(|g| *g != 0xffff && *g != 0)
    }
}
pub(super) fn installed() -> Vec<String> {
    let mut names = Vec::<String>::new();
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return names;
        }
        let lf = LOGFONTW {
            lfCharSet: DEFAULT_CHARSET,
            ..Default::default()
        };
        EnumFontFamiliesExW(
            dc,
            &lf,
            Some(collect),
            (&mut names as *mut Vec<String>) as isize,
            0,
        );
        DeleteDC(dc);
    }
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names.retain(|name| readable(name));
    names
}
pub(super) fn load(store: &desktop_storage::WorkspaceStore) -> Result<(), String> {
    let saved = store
        .preference(KEY)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if saved.is_empty() || saved == super::assets::UI_FONT {
        set(String::new());
    } else {
        set(installed()
            .into_iter()
            .find(|name| name == &saved)
            .unwrap_or_default());
    }
    Ok(())
}
pub(super) fn save(store: &desktop_storage::WorkspaceStore, name: &str) -> Result<(), String> {
    if name != super::assets::UI_FONT && !installed().iter().any(|candidate| candidate == name) {
        return Err("该字体未安装，或不支持所需的中英文字符".into());
    }
    store
        .save_preference(KEY, name)
        .map_err(|e| e.to_string())?;
    set(name.into());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn downlevel_icon_font_covers_all_application_symbols() {
        assert_eq!(choose_icon_family(|_| false), "Segoe MDL2 Assets");
        assert_eq!(choose_icon_family(|_| true), "Segoe Fluent Icons");
        assert!(icon_coverage("Segoe MDL2 Assets").unwrap());
        assert!(!icon_coverage("LucidDesk nonexistent icon font").unwrap());
        assert!(icon_coverage(icon_family()).unwrap());
    }
    #[test]
    fn regular_faces_exclude_weight_variants_and_italics() {
        for weight in [100, 200, 300, 400, 500, 600, 700, 800, 900] {
            for italic in [0, 1] {
                let font = LOGFONTW {
                    lfWeight: weight,
                    lfItalic: italic,
                    ..Default::default()
                };
                assert_eq!(regular_face(&font), weight == 400 && italic == 0);
            }
        }
    }
    #[test]
    fn font_candidates_exclude_symbols_vertical_faces_and_missing_glyphs() {
        let names = installed();
        assert!(
            names
                .iter()
                .any(|name| name == super::super::assets::UI_FONT)
        );
        assert!(
            names
                .iter()
                .all(|name| !name.starts_with('@') && readable(name))
        );
        let store = desktop_storage::WorkspaceStore::open_in_memory().unwrap();
        assert!(save(&store, "LucidDesk nonexistent font 82947").is_err());
        assert!(store.preference(KEY).unwrap().is_none());
    }
    #[test]
    fn font_switch_updates_live_layout_persists_and_missing_fonts_fall_back() {
        let _sta = desktop_shell::ShellApartment::initialize_sta().unwrap();
        struct Restore(String);
        impl Drop for Restore {
            fn drop(&mut self) {
                set(self.0.clone());
            }
        }
        let _restore = Restore(family());
        set(super::super::assets::UI_FONT.into());
        let names = installed();
        let alternative = names
            .iter()
            .find(|name| name.as_str() == "SimSun")
            .or_else(|| {
                names
                    .iter()
                    .find(|name| name.as_str() != super::super::assets::UI_FONT)
            })
            .expect("another installed Chinese font");
        let mut state = super::super::tests::test_state();
        state
            .workspace
            .set_appearance(desktop_core::PanelTheme::Dark, desktop_core::Backdrop::Mica);
        let model = super::super::create_model(&state, desktop_core::PanelId::new(1)).unwrap();
        let mut renderer = super::super::render::Renderer::new().unwrap();
        let before = renderer.pixels(420, 300, 1.0, &model).unwrap();
        let store = desktop_storage::WorkspaceStore::open_in_memory().unwrap();
        save(&store, alternative).unwrap();
        assert_eq!(family(), *alternative);
        assert_ne!(before, renderer.pixels(420, 300, 1.0, &model).unwrap());
        assert_eq!(
            store.preference(KEY).unwrap().as_deref(),
            Some(alternative.as_str())
        );
        set(String::new());
        load(&store).unwrap();
        assert_eq!(family(), *alternative);
        store.save_preference(KEY, "Removed font 928471").unwrap();
        load(&store).unwrap();
        assert_eq!(family(), super::super::assets::UI_FONT);
    }
}
