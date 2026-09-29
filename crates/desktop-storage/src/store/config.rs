//! Editable global preferences. The application preference API is adapted to named TOML fields.
use super::{StoreError, WorkspaceStore};
use std::{cell::RefCell, collections::BTreeMap, io::Write};
use std::{
    fmt,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, value};

pub(super) const KEYS: &[&str] = &[
    "language",
    "appearance",
    "pane_options",
    "solid_style",
    "material_strength_acrylic",
    "material_strength_mica",
    "search_enabled",
    "everything",
    "search_hotkey",
    "peek",
];
const DEFAULTS: &str = r##"# LucidDesk global settings. Reload from Settings after editing.
config_version = 1
language = "system"

[appearance]
theme = "system"
material = "mica"

[appearance.acrylic]
strength = 50

[appearance.mica]
strength = 50

[appearance.translucent]
opacity = 0.86

[appearance.solid]
color = "#24364B"
opacity = 0.85

[panel_defaults]
corner_radius = 6.0
grid_scale = 100.0
border = true
snap = true
text = "auto"
text_protection = false

[search]
enabled = false
shortcut = "Ctrl+Shift+Space"
everything_path = ""

[preview]
enabled = false
provider = "peek"
shortcut = "Space"
peek_path = ""
quicklook_path = ""
"##;

fn error(message: impl Into<String>) -> StoreError {
    StoreError::InvalidData(message.into())
}
fn io(error: impl fmt::Display) -> StoreError {
    self::error(format!("config.toml: {error}"))
}
fn get<'a>(doc: &'a DocumentMut, path: &[&str]) -> Option<&'a Item> {
    let mut item = doc.as_item();
    for part in path {
        item = item.get(part)?;
    }
    Some(item)
}
fn set(doc: &mut DocumentMut, path: &[&str], new: Item) {
    let mut item = doc.as_item_mut();
    for part in path {
        item = &mut item[*part];
    }
    if let (Some(old), Some(next)) = (item.as_value(), new.as_value()) {
        let mut next = next.clone();
        *next.decor_mut() = old.decor().clone();
        *item = Item::Value(next);
    } else {
        *item = new;
    }
}
fn field<'a>(doc: &'a DocumentMut, defaults: &'a DocumentMut, path: &[&str]) -> &'a Item {
    get(doc, path).unwrap_or_else(|| get(defaults, path).unwrap())
}
fn string(doc: &DocumentMut, defaults: &DocumentMut, path: &[&str]) -> Result<String, StoreError> {
    field(doc, defaults, path)
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| error(format!("{} must be a string", path.join("."))))
}
fn boolean(doc: &DocumentMut, defaults: &DocumentMut, path: &[&str]) -> Result<bool, StoreError> {
    field(doc, defaults, path)
        .as_bool()
        .ok_or_else(|| error(format!("{} must be true or false", path.join("."))))
}
fn number(
    doc: &DocumentMut,
    defaults: &DocumentMut,
    path: &[&str],
    max: f64,
) -> Result<f64, StoreError> {
    let item = field(doc, defaults, path);
    let n = item
        .as_float()
        .or_else(|| item.as_integer().map(|n| n as f64));
    n.filter(|n| n.is_finite() && (0.0..=max).contains(n))
        .ok_or_else(|| error(format!("{} must be between 0 and {max}", path.join("."))))
}
fn shortcut(raw: &str, search: bool) -> Result<(u16, u8), StoreError> {
    let mut parts: Vec<_> = raw.split('+').map(str::trim).collect();
    let key = parts.pop().unwrap_or_default();
    let key = if key.eq_ignore_ascii_case("Space") {
        32
    } else if key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric() {
        u16::from(key.as_bytes()[0].to_ascii_uppercase())
    } else if let Some(n) = key
        .strip_prefix('F')
        .and_then(|n| n.parse::<u16>().ok())
        .filter(|n| (1..=24).contains(n))
    {
        111 + n
    } else {
        return Err(error("invalid shortcut key"));
    };
    let mut bits = 0;
    for part in parts {
        let bit = match part.to_ascii_lowercase().as_str() {
            "ctrl" => 1,
            "shift" => 2,
            "alt" => 4,
            _ => return Err(error("invalid shortcut modifier")),
        };
        if bits & bit != 0 {
            return Err(error("duplicate shortcut modifier"));
        }
        bits |= bit;
    }
    if (bits & 4 != 0 && matches!(key, 32 | 115)) || (search && (bits & 5 == 0 || key == 123)) {
        return Err(error("shortcut is reserved or missing Ctrl/Alt"));
    }
    if (bits == 1 && matches!(key, 32 | 65 | 67 | 86 | 88))
        || (bits == 0 && matches!(key, 113 | 116))
        || (bits == 2 && key == 121)
    {
        return Err(error("shortcut conflicts with a file operation"));
    }
    Ok((key, bits))
}
fn shortcut_label(key: &str, bits: &str) -> Result<String, StoreError> {
    let key: u16 = key.parse().map_err(io)?;
    let bits: u8 = bits.parse().map_err(io)?;
    let mut parts = Vec::new();
    for (bit, name) in [(1, "Ctrl"), (2, "Shift"), (4, "Alt")] {
        if bits & bit != 0 {
            parts.push(name.to_owned());
        }
    }
    parts.push(match key {
        32 => "Space".into(),
        112..=135 => format!("F{}", key - 111),
        48..=57 | 65..=90 => char::from_u32(u32::from(key)).unwrap().to_string(),
        _ => return Err(error("invalid shortcut key")),
    });
    Ok(parts.join("+"))
}

pub(super) fn decode(doc: &DocumentMut) -> Result<BTreeMap<String, String>, StoreError> {
    let defaults: DocumentMut = DEFAULTS.parse().unwrap();
    if get(doc, &["config_version"]).and_then(Item::as_integer) != Some(1) {
        return Err(error("unsupported config_version (expected 1)"));
    }
    // A scalar in place of a table is an error, not a missing configuration section.
    for path in [
        &["appearance"][..],
        &["panel_defaults"],
        &["search"],
        &["preview"],
        &["appearance", "acrylic"],
        &["appearance", "mica"],
        &["appearance", "solid"],
        &["appearance", "translucent"],
    ] {
        if get(doc, path).is_some_and(|i| !i.is_table_like()) {
            return Err(error(format!("{} must be a table", path.join("."))));
        }
    }
    let s = |p: &[&str]| string(doc, &defaults, p);
    let b = |p: &[&str]| boolean(doc, &defaults, p);
    let n = |p: &[&str], max| number(doc, &defaults, p, max);
    for path in [
        &["search", "everything_path"][..],
        &["preview", "peek_path"],
        &["preview", "quicklook_path"],
    ] {
        if s(path)?.contains(['\n', '\r', '\0']) {
            return Err(error(format!(
                "{} must be a single-line path",
                path.join(".")
            )));
        }
    }
    let theme = s(&["appearance", "theme"])?;
    if !["system", "light", "dark"].contains(&theme.as_str()) {
        return Err(error("invalid appearance.theme"));
    }
    let text = s(&["panel_defaults", "text"])?;
    if !["auto", "light", "dark"].contains(&text.as_str()) {
        return Err(error("invalid panel_defaults.text"));
    }
    let mut map = BTreeMap::new();
    let language = s(&["language"])?;
    validate_language(&language)?;
    map.insert("language".into(), language);
    for material in ["acrylic", "mica"] {
        let strength = n(&["appearance", material, "strength"], 100.0)?;
        if strength.fract() != 0.0 {
            return Err(error("material strength must be an integer"));
        }
        map.insert(
            format!("material_strength_{material}"),
            strength.to_string(),
        );
    }
    let raw_color = s(&["appearance", "solid", "color"])?;
    let color = raw_color
        .strip_prefix('#')
        .filter(|v| v.len() == 6)
        .and_then(|v| u32::from_str_radix(v, 16).ok())
        .ok_or_else(|| error("appearance.solid.color must be #RRGGBB"))?;
    let opacity = n(&["appearance", "solid", "opacity"], 1.0)?;
    map.insert("solid_style".into(), format!("{color}|{opacity}"));
    let translucent = n(&["appearance", "translucent", "opacity"], 1.0)?;
    let material = s(&["appearance", "material"])?;
    let appearance = match material.as_str() {
        "acrylic" | "mica" => format!(
            "{theme}|{material}_tuned|{}|",
            map[&format!("material_strength_{material}")]
                .parse::<f64>()
                .unwrap()
                / 100.0
        ),
        "mica_alt" => format!("{theme}|mica_alt|1|"),
        "solid" => format!("{theme}|solid|{opacity}|{color}"),
        "translucent" => format!("{theme}|translucent|{translucent}|"),
        _ => return Err(error("invalid appearance.material")),
    };
    map.insert("appearance".into(), appearance);
    map.insert(
        "pane_options".into(),
        format!(
            "{}|{}|{}|{text}|{}|{}",
            n(&["panel_defaults", "corner_radius"], 24.0)?,
            b(&["panel_defaults", "border"])?,
            b(&["panel_defaults", "snap"])?,
            b(&["panel_defaults", "text_protection"])?,
            grid_dimension(doc, &defaults, "grid_scale", desktop_core::PaneOptions::GRID_SCALE_RANGE)?
        ),
    );
    map.insert(
        "search_enabled".into(),
        if b(&["search", "enabled"])? { "1" } else { "0" }.into(),
    );
    map.insert(
        "everything".into(),
        format!("0\n{}", s(&["search", "everything_path"])?),
    );
    let (key, bits) = shortcut(&s(&["search", "shortcut"])?, true)?;
    map.insert("search_hotkey".into(), format!("{key}:{bits}"));
    let provider = match s(&["preview", "provider"])?.as_str() {
        "peek" => "Peek",
        "quicklook" => "QuickLook",
        _ => return Err(error("invalid preview.provider")),
    };
    let (key, bits) = shortcut(&s(&["preview", "shortcut"])?, false)?;
    map.insert(
        "peek".into(),
        format!(
            "v2\n{}\n{key}\n{bits}\n{provider}\n{}\n{}",
            u8::from(b(&["preview", "enabled"])?),
            s(&["preview", "peek_path"])?,
            s(&["preview", "quicklook_path"])?
        ),
    );
    Ok(map)
}

fn grid_dimension(doc: &DocumentMut, defaults: &DocumentMut, name: &str, range: (f32, f32)) -> Result<f64, StoreError> {
    let value = number(doc, defaults, &["panel_defaults", name], f64::from(range.1))?;
    if value < f64::from(range.0) { return Err(error(format!("panel_defaults.{name} is too small"))); }
    Ok(value)
}

fn validate_language(value: &str) -> Result<(), StoreError> {
    if ["system", "zh-CN", "zh-TW", "en-US", "ja-JP", "ko-KR", "de-DE", "ru-RU"].contains(&value) { Ok(()) }
    else { Err(error("unsupported language")) }
}

fn update(doc: &mut DocumentMut, key: &str, raw: &str) -> Result<(), StoreError> {
    let parts: Vec<_> = raw.split('|').collect();
    let f = |s: &str| s.parse::<f64>().map_err(io);
    let flag = |s: &str| s.parse::<bool>().map_err(io);
    match key {
        "language" => { validate_language(raw)?; set(doc, &["language"], value(raw)); }
        "pane_options" if (3..=5).contains(&parts.len()) || parts.len() == 6 => {
            let radius = match parts[0] {
                "true" => 7.0,
                "false" => 0.0,
                v => f(v)?,
            };
            for (name, v) in [
                ("corner_radius", value(radius)),
                ("grid_scale", value(parts.get(5).map(|v| f(v)).transpose()?.unwrap_or(100.0))),
                ("border", value(flag(parts[1])?)),
                ("snap", value(flag(parts[2])?)),
                ("text", value(*parts.get(3).unwrap_or(&"auto"))),
                (
                    "text_protection",
                    value(parts.get(4).map(|v| flag(v)).transpose()?.unwrap_or(false)),
                ),
            ] {
                set(doc, &["panel_defaults", name], v);
            }
        }
        "appearance" if (3..=4).contains(&parts.len()) => {
            set(doc, &["appearance", "theme"], value(parts[0]));
            let material = parts[1].trim_end_matches("_tuned");
            set(doc, &["appearance", "material"], value(material));
            if parts[1].ends_with("_tuned") && material != "mica_alt" {
                set(
                    doc,
                    &["appearance", material, "strength"],
                    value((f(parts[2])? * 100.0).round() as i64),
                );
            }
            if ["solid", "translucent"].contains(&material) {
                set(
                    doc,
                    &["appearance", material, "opacity"],
                    value(f(parts[2])?),
                );
            }
            if material == "solid" {
                let color: u32 = parts
                    .get(3)
                    .ok_or_else(|| error("missing solid color"))?
                    .parse()
                    .map_err(io)?;
                set(
                    doc,
                    &["appearance", "solid", "color"],
                    value(format!("#{color:06X}")),
                );
            }
        }
        "solid_style" if parts.len() == 2 => {
            let color: u32 = parts[0].parse().map_err(io)?;
            set(
                doc,
                &["appearance", "solid", "color"],
                value(format!("#{color:06X}")),
            );
            set(
                doc,
                &["appearance", "solid", "opacity"],
                value(f(parts[1])?),
            );
        }
        "material_strength_acrylic" | "material_strength_mica" => set(
            doc,
            &[
                "appearance",
                key.trim_start_matches("material_strength_"),
                "strength",
            ],
            value(raw.parse::<i64>().map_err(io)?),
        ),
        "search_enabled" if ["0", "1"].contains(&raw) => {
            set(doc, &["search", "enabled"], value(raw == "1"));
        }
        "everything" => {
            let (_, path) = raw
                .split_once('\n')
                .ok_or_else(|| error("invalid legacy Everything settings"))?;
            set(doc, &["search", "everything_path"], value(path));
        }
        "search_hotkey" => {
            let (key, bits) = raw
                .split_once(':')
                .ok_or_else(|| error("invalid shortcut"))?;
            set(
                doc,
                &["search", "shortcut"],
                value(shortcut_label(key, bits)?),
            );
        }
        "peek" => {
            let (raw, v2) = raw.strip_prefix("v2\n").map_or((raw, false), |r| (r, true));
            let p: Vec<_> = raw.splitn(if v2 { 6 } else { 4 }, '\n').collect();
            if p.len() != if v2 { 6 } else { 4 } || !["0", "1"].contains(&p[0]) {
                return Err(error("invalid preview settings"));
            }
            set(doc, &["preview", "enabled"], value(p[0] == "1"));
            set(
                doc,
                &["preview", "shortcut"],
                value(shortcut_label(p[1], p[2])?),
            );
            set(
                doc,
                &["preview", "provider"],
                value(if v2 {
                    p[3].to_ascii_lowercase()
                } else {
                    "peek".into()
                }),
            );
            set(
                doc,
                &["preview", "peek_path"],
                value(p[if v2 { 4 } else { 3 }]),
            );
            set(
                doc,
                &["preview", "quicklook_path"],
                value(if v2 { p[5] } else { "" }),
            );
        }
        _ => return Err(error(format!("invalid configuration value for {key}"))),
    }
    Ok(())
}

pub(super) struct ConfigFile {
    pub path: PathBuf,
    pub source: String,
    pub doc: DocumentMut,
    pub values: BTreeMap<String, String>,
    pub changes: u64,
}
impl ConfigFile {
    pub fn parse(path: PathBuf, source: String) -> Result<Self, StoreError> {
        let doc = source.parse::<DocumentMut>().map_err(io)?;
        let values = decode(&doc)?;
        Ok(Self {
            path,
            source,
            doc,
            values,
            changes: 0,
        })
    }
    pub fn check_disk(&self) -> Result<(), StoreError> {
        if std::fs::read_to_string(&self.path).map_err(io)? != self.source {
            return Err(error(
                "config.toml 已被外部修改，请先重新加载配置后再保存。",
            ));
        }
        Ok(())
    }
    pub fn save(&mut self, updates: &[(&str, String)]) -> Result<(), StoreError> {
        if updates.iter().all(|(k, v)| self.values.get(*k) == Some(v)) {
            return Ok(());
        }
        let mut doc = self.doc.clone();
        for (key, raw) in updates {
            update(&mut doc, key, raw)?;
        }
        let values = decode(&doc)?;
        let source = doc.to_string();
        if source == self.source {
            return Ok(());
        }
        self.check_disk()?;
        atomic_write(&self.path, &source)?;
        self.doc = doc;
        self.values = values;
        self.source = source;
        self.changes += 1;
        Ok(())
    }
}

pub(super) fn atomic_write(path: &Path, source: &str) -> Result<(), StoreError> {
    let mut file = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| error("missing config directory"))?,
    )
    .map_err(io)?;
    file.write_all(source.as_bytes()).map_err(io)?;
    file.as_file().sync_all().map_err(io)?;
    file.persist(path).map_err(io)?;
    Ok(())
}

impl WorkspaceStore {
    pub(super) fn attach_config(&mut self, path: &Path) -> Result<(), StoreError> {
        if let Some(source) = self.preference("pending_config")? {
            ConfigFile::parse(path.to_owned(), source.clone())?;
            atomic_write(path, &source)?;
            self.connection
                .execute("DELETE FROM metadata WHERE key='pending_config'", [])?;
        }
        let config = match std::fs::read_to_string(path) {
            Ok(source) => ConfigFile::parse(path.to_owned(), source)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = ConfigFile::parse(path.to_owned(), DEFAULTS.to_owned())?;
                // Never overwrite a file created by another process while creating the initial config.
                let mut file =
                    tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(io)?;
                file.write_all(config.source.as_bytes()).map_err(io)?;
                file.as_file().sync_all().map_err(io)?;
                file.persist_noclobber(path).map_err(io)?;
                config
            }
            Err(e) => return Err(io(e)),
        };
        self.config = Some(RefCell::new(config));
        Ok(())
    }
    /// Reloads a valid externally edited configuration; failures preserve the active settings.
    /// # Errors
    /// Reports read errors, unsupported versions and invalid configuration fields.
    pub fn reload_config(&self) -> Result<(), StoreError> {
        let previous = self.change_count();
        if let Some(config) = &self.config {
            let mut config = config.borrow_mut();
            let mut next = ConfigFile::parse(
                config.path.clone(),
                std::fs::read_to_string(&config.path).map_err(io)?,
            )?;
            next.changes = config.changes + 1;
            *config = next;
        }
        self.notify_change(previous);
        Ok(())
    }
    #[must_use]
    pub fn config_path(&self) -> Option<PathBuf> {
        self.config.as_ref().map(|c| c.borrow().path.clone())
    }
}
