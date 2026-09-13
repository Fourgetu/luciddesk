use desktop_core::{
    Backdrop, DesktopItem, DesktopPlacement, GridPosition, MonitorId, Panel, PanelId, PointDip,
    RectDip, ShellIdentity, Workspace,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: i64 = 11;
mod recovery;

fn encode_panel_text(text: desktop_core::PanelText) -> &'static str {
    match text {
        desktop_core::PanelText::Auto => "auto",
        desktop_core::PanelText::Light => "light",
        desktop_core::PanelText::Dark => "dark",
    }
}

pub struct WorkspaceStore {
    connection: Connection,
}

impl WorkspaceStore {
    /// Reads an optional application preference.
    /// # Errors
    /// Returns an error if the database query fails.
    pub fn preference(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row("SELECT value FROM metadata WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Saves an application preference atomically.
    /// # Errors
    /// Returns an error if the database update fails.
    pub fn save_preference(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO metadata(key,value) VALUES (?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value
             WHERE metadata.value != excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Opens or creates a `LucidPane` workspace database.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be opened or initialized.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
        let version: Option<String> = connection
            .query_row(
                "SELECT value FROM metadata WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap_or(None);
        if matches!(version.as_deref(), Some("8" | "9" | "10")) {
            let backup = recovery::unique_backup_path(path, "before-v11");
            connection.backup(rusqlite::MAIN_DB, backup, None)?;
        }
        Self::from_connection(connection)
    }

    /// Creates an in-memory workspace database for tests and temporary sessions.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema cannot be initialized.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let connection = Connection::open_in_memory()?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, StoreError> {
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        initialize_schema(&connection)?;
        Ok(Self { connection })
    }

    /// Loads the complete workspace.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid persisted data or a database failure.
    pub fn load_workspace(&self) -> Result<Workspace, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id, title, x, y, width, height, \
             collapsed, locked, backdrop_kind, opacity, color FROM panels ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            let raw_id: i64 = row.get(0)?;
            let backdrop_kind: String = row.get(8)?;
            let opacity: Option<f32> = row.get(9)?;
            Ok(PersistedPanel {
                id: raw_id,
                title: row.get(1)?,
                rect: RectDip::new(row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?),
                collapsed: row.get(6)?,
                locked: row.get(7)?,
                backdrop_kind,
                opacity,
                color: row.get(10)?,
            })
        })?;

        let mut panels = Vec::new();
        for row in rows {
            panels.push(row?.into_panel()?);
        }
        drop(statement);
        for panel in &mut panels {
            panel.set_folder_list(
                self.preference(&format!("panel_folder_view:{}", panel.id().get()))?
                    .as_deref()
                    != Some("icons"),
            );
            panel.set_folder(
                self.preference(&format!("panel_folder:{}", panel.id().get()))?
                    .map(PathBuf::from),
            );
            panel.set_search(
                self.preference(&format!("panel_search:{}", panel.id().get()))?
                    .as_deref()
                    == Some("true"),
            );
            let panel_id = i64::try_from(panel.id().get())
                .map_err(|_| StoreError::InvalidData("panel id exceeds SQLite range".into()))?;
            let auto_hide = self
                .connection
                .query_row(
                    "SELECT auto_hide FROM panel_behavior WHERE panel_id = ?1",
                    [panel_id],
                    |row| row.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(false);
            panel.set_auto_hide(auto_hide);
            let theme = self
                .connection
                .query_row(
                    "SELECT theme FROM panel_theme WHERE panel_id = ?1",
                    [panel_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            panel.set_theme(match theme.as_deref() {
                Some("light") => desktop_core::PanelTheme::Light,
                Some("dark") => desktop_core::PanelTheme::Dark,
                _ => desktop_core::PanelTheme::System,
            });
            panel.set_always_on_top(
                self.connection
                    .query_row(
                        "SELECT always_on_top FROM panel_layer WHERE panel_id = ?1",
                        [panel_id],
                        |row| row.get::<_, bool>(0),
                    )
                    .optional()?
                    .unwrap_or(false),
            );
        }
        let mut workspace = Workspace::from_panels(panels)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        workspace.reconcile_desktop_items(load_desktop_items(&self.connection)?);
        let appearance = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key = 'appearance'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(value) = appearance {
            let parts: Vec<_> = value.split('|').collect();
            if parts.len() != 3 && parts.len() != 4 {
                return Err(StoreError::InvalidData("invalid appearance".into()));
            }
            let theme = match parts[0] {
                "light" => desktop_core::PanelTheme::Light,
                "dark" => desktop_core::PanelTheme::Dark,
                _ => desktop_core::PanelTheme::System,
            };
            let opacity = parts[2]
                .parse()
                .map_err(|_| StoreError::InvalidData("invalid opacity".into()))?;
            let color = parts
                .get(3)
                .filter(|v| !v.is_empty())
                .map(|v| v.parse::<u32>())
                .transpose()
                .map_err(|_| StoreError::InvalidData("invalid color".into()))?;
            let backdrop = decode_backdrop(parts[1], Some(opacity), color)?;
            workspace.set_appearance_defaults(theme, backdrop);
        }
        let options = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key = 'pane_options'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(value) = options {
            let parts = value.split('|').collect::<Vec<_>>();
            let [radius, border, snap, text @ ..] = parts.as_slice() else {
                return Err(StoreError::InvalidData("invalid pane options".into()));
            };
            let invalid = || StoreError::InvalidData("invalid pane options".into());
            let corner_radius = match *radius {
                "true" => 7.0,
                "false" => 0.0,
                value => value.parse::<f32>().map_err(|_| invalid())?,
            };
            if !corner_radius.is_finite()
                || !(0.0..=desktop_core::PaneOptions::MAX_CORNER_RADIUS).contains(&corner_radius)
            {
                return Err(invalid());
            }
            workspace.set_pane_options(desktop_core::PaneOptions {
                corner_radius,
                border: border.parse().map_err(|_| invalid())?,
                snap: snap.parse().map_err(|_| invalid())?,
                text: match text {
                    [] | ["auto"] | ["auto", _] => desktop_core::PanelText::Auto,
                    ["light"] | ["light", _] => desktop_core::PanelText::Light,
                    ["dark"] | ["dark", _] => desktop_core::PanelText::Dark,
                    _ => return Err(invalid()),
                },
                text_protection: match text {
                    [] | [_] => false,
                    [_, enabled] => enabled.parse().map_err(|_| invalid())?,
                    _ => return Err(invalid()),
                },
            });
        }
        Ok(workspace)
    }

    /// Updates only global pane options.
    ///
    /// # Errors
    /// Returns an error if the metadata update fails.
    pub fn save_pane_options(&self, options: desktop_core::PaneOptions) -> Result<(), StoreError> {
        self.save_preference(
            "pane_options",
            &format!(
                "{}|{}|{}|{}|{}",
                options.corner_radius,
                options.border,
                options.snap,
                encode_panel_text(options.text),
                options.text_protection
            ),
        )
    }

    /// Replaces the persisted workspace in one transaction.
    ///
    /// # Errors
    /// Returns an error when serialization or commit fails.
    pub fn save_workspace(&mut self, workspace: &Workspace) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM metadata WHERE key GLOB 'panel_folder:*'", [])?;
        transaction.execute("DELETE FROM metadata WHERE key GLOB 'panel_search:*'", [])?;
        transaction.execute(
            "DELETE FROM metadata WHERE key GLOB 'panel_folder_view:*'",
            [],
        )?;
        for panel in workspace.panels() {
            if panel.is_search() {
                transaction.execute(
                    "INSERT INTO metadata(key,value) VALUES (?1,'true')",
                    [format!("panel_search:{}", panel.id().get())],
                )?;
            }
            if let Some(folder) = panel.folder() {
                transaction.execute(
                    "INSERT INTO metadata(key,value) VALUES (?1,?2)",
                    params![
                        format!("panel_folder_view:{}", panel.id().get()),
                        if panel.folder_list() { "list" } else { "icons" }
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO metadata(key,value) VALUES (?1,?2)",
                    params![
                        format!("panel_folder:{}", panel.id().get()),
                        folder.to_string_lossy()
                    ],
                )?;
            }
        }
        let options = workspace.pane_options();
        transaction.execute(
            "INSERT OR REPLACE INTO metadata(key,value) VALUES ('pane_options',?1)",
            [format!(
                "{}|{}|{}|{}|{}",
                options.corner_radius,
                options.border,
                options.snap,
                encode_panel_text(options.text),
                options.text_protection
            )],
        )?;
        transaction.execute("DELETE FROM metadata WHERE key = 'appearance'", [])?;
        if let Some((theme, backdrop)) = workspace.appearance() {
            let theme = match theme {
                desktop_core::PanelTheme::System => "system",
                desktop_core::PanelTheme::Light => "light",
                desktop_core::PanelTheme::Dark => "dark",
            };
            let (kind, opacity, color) = encode_backdrop(backdrop);
            decode_backdrop(kind, opacity, color)?;
            if let (Some(key), Some(strength)) = (backdrop.strength_key(), backdrop.strength()) {
                transaction.execute(
                    "INSERT OR REPLACE INTO metadata(key,value) VALUES (?1,?2)",
                    [key, &strength.to_string()],
                )?;
            }
            if let Backdrop::Solid { color, opacity } = backdrop {
                transaction.execute(
                    "INSERT OR REPLACE INTO metadata(key,value) VALUES ('solid_style',?1)",
                    [format!("{color}|{opacity}")],
                )?;
            }

            transaction.execute(
                "INSERT INTO metadata(key,value) VALUES ('appearance',?1)",
                [format!(
                    "{theme}|{kind}|{}|{}",
                    opacity.unwrap_or(1.0),
                    color.map(|v| v.to_string()).unwrap_or_default()
                )],
            )?;
        }
        transaction.execute("DELETE FROM desktop_items", [])?;
        transaction.execute("DELETE FROM panels", [])?;
        for panel in workspace.panels() {
            insert_panel(&transaction, panel)?;
        }
        transaction.execute(
            "DELETE FROM monitor_layouts WHERE panel_id NOT IN (SELECT id FROM panels)",
            [],
        )?;
        insert_desktop_items(&transaction, workspace.desktop_items())?;
        transaction.execute("DELETE FROM metadata WHERE key GLOB 'panel_folder_sort:*' AND substr(key,19) NOT IN (SELECT CAST(id AS TEXT) FROM panels)", [])?;
        transaction.commit()?;
        Ok(())
    }
}

fn initialize_schema(connection: &Connection) -> Result<(), StoreError> {
    let has_tables: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%')",
        [], |row| row.get(0),
    )?;
    if has_tables {
        let version: Option<String> = connection
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if matches!(version.as_deref(), Some("8" | "9" | "10")) {
            let transaction = connection.unchecked_transaction()?;
            if version.as_deref() == Some("8") {
                recovery::migrate_v8(&transaction)?;
            }
            if version.as_deref() != Some("10") {
                transaction.execute_batch(recovery::LAYOUT_SCHEMA)?;
            }
            transaction.execute_batch("ALTER TABLE panels ADD COLUMN color INTEGER;")?;
            transaction.execute(
                "UPDATE metadata SET value=?1 WHERE key='schema_version'",
                [SCHEMA_VERSION.to_string()],
            )?;
            transaction.commit()?;
            return Ok(());
        }
        if version.as_deref() != Some(SCHEMA_VERSION.to_string().as_str()) {
            return Err(StoreError::InvalidData(format!(
                "配置版本 {} 暂不支持；当前支持 v8、v9、v10 升级至 v{SCHEMA_VERSION}。原配置未修改，请保留数据库用于迁移",
                version.as_deref().unwrap_or("missing"),
            )));
        }
        return Ok(());
    }
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS metadata (
             key TEXT PRIMARY KEY NOT NULL,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS panels (
             id INTEGER PRIMARY KEY NOT NULL,
             title TEXT NOT NULL,
             x REAL NOT NULL,
             y REAL NOT NULL,
             width REAL NOT NULL,
             height REAL NOT NULL,
             collapsed INTEGER NOT NULL,
             locked INTEGER NOT NULL,
             backdrop_kind TEXT NOT NULL,
             opacity REAL,
             color INTEGER
         );
         CREATE TABLE IF NOT EXISTS panel_theme (
             panel_id INTEGER PRIMARY KEY REFERENCES panels(id) ON DELETE CASCADE,
             theme TEXT NOT NULL DEFAULT 'system'
         );
         CREATE TABLE IF NOT EXISTS panel_layer (
             panel_id INTEGER PRIMARY KEY REFERENCES panels(id) ON DELETE CASCADE,
             always_on_top INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS panel_behavior (
             panel_id INTEGER PRIMARY KEY REFERENCES panels(id) ON DELETE CASCADE,
             auto_hide INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS desktop_items (
             identity_key TEXT PRIMARY KEY NOT NULL,
             identity_kind TEXT NOT NULL,
             identity_value TEXT NOT NULL,
             volume_id TEXT,
             file_id TEXT,
             display_name TEXT NOT NULL,
             placement_kind TEXT NOT NULL,
             monitor_id TEXT,
             x REAL,
             y REAL,
             pane_id INTEGER,
             grid_column INTEGER,
             grid_row INTEGER
         );",
    )?;

    transaction.execute_batch(recovery::LAYOUT_SCHEMA)?;
    transaction.execute(
        "INSERT INTO metadata(key, value) VALUES ('schema_version', ?1)",
        [SCHEMA_VERSION.to_string()],
    )?;
    transaction.commit()?;
    Ok(())
}

fn insert_panel(transaction: &Transaction<'_>, panel: &Panel) -> Result<(), StoreError> {
    let id = i64::try_from(panel.id().get())
        .map_err(|_| StoreError::InvalidData("panel id exceeds SQLite range".into()))?;
    let (backdrop_kind, opacity, color) = encode_backdrop(panel.backdrop());
    decode_backdrop(backdrop_kind, opacity, color)?;
    let rect = panel.rect();
    transaction.execute(
        "INSERT INTO panels(
             id, title, x, y, width, height,
             collapsed, locked, backdrop_kind, opacity, color
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id,
            panel.title(),
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            panel.collapsed(),
            panel.locked(),
            backdrop_kind,
            opacity,
            color,
        ],
    )?;
    transaction.execute(
        "INSERT INTO panel_behavior(panel_id, auto_hide) VALUES (?1, ?2)",
        params![id, panel.auto_hide()],
    )?;
    transaction.execute(
        "INSERT INTO panel_layer(panel_id, always_on_top) VALUES (?1, ?2)",
        params![id, panel.always_on_top()],
    )?;
    transaction.execute(
        "INSERT INTO panel_theme(panel_id, theme) VALUES (?1, ?2)",
        params![
            id,
            match panel.theme() {
                desktop_core::PanelTheme::System => "system",
                desktop_core::PanelTheme::Light => "light",
                desktop_core::PanelTheme::Dark => "dark",
            }
        ],
    )?;
    Ok(())
}

fn load_desktop_items(connection: &Connection) -> Result<Vec<DesktopItem>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT identity_kind, identity_value, volume_id, file_id, display_name,
                placement_kind, monitor_id, x, y, pane_id, grid_column, grid_row
         FROM desktop_items ORDER BY identity_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(PersistedDesktopItem {
            identity_kind: row.get(0)?,
            identity_value: row.get(1)?,
            volume_id: row.get(2)?,
            file_id: row.get(3)?,
            display_name: row.get(4)?,
            placement_kind: row.get(5)?,
            monitor_id: row.get(6)?,
            x: row.get(7)?,
            y: row.get(8)?,
            pane_id: row.get(9)?,
            grid_column: row.get(10)?,
            grid_row: row.get(11)?,
        })
    })?;
    rows.map(|row| {
        row.map_err(StoreError::from)
            .and_then(PersistedDesktopItem::into_item)
    })
    .collect()
}

fn insert_desktop_items(
    transaction: &Transaction<'_>,
    items: &[DesktopItem],
) -> Result<(), StoreError> {
    for item in items {
        let (identity_kind, identity_value, volume_id, file_id) = match item.identity() {
            ShellIdentity::FileSystem {
                path,
                volume_id,
                file_id,
            } => (
                "filesystem",
                path.as_os_str().to_string_lossy().into_owned(),
                volume_id.map(|value| value.to_string()),
                file_id.map(|value| value.to_string()),
            ),
            ShellIdentity::Namespace { parsing_name } => {
                ("namespace", parsing_name.clone(), None, None)
            }
        };
        let (placement_kind, monitor_id, x, y, pane_id, grid_column, grid_row) =
            match item.placement() {
                DesktopPlacement::FreeDesktop { monitor, position } => (
                    "free",
                    Some(monitor.as_str().to_string()),
                    Some(f64::from(position.x)),
                    Some(f64::from(position.y)),
                    None,
                    None,
                    None,
                ),
                DesktopPlacement::Pane { pane_id, position } => (
                    "pane",
                    None,
                    None,
                    None,
                    Some(i64::try_from(pane_id.get()).map_err(|_| {
                        StoreError::InvalidData("pane id exceeds SQLite range".into())
                    })?),
                    Some(i64::from(position.column)),
                    Some(i64::from(position.row)),
                ),
            };
        transaction.execute(
            "INSERT INTO desktop_items(
                 identity_key, identity_kind, identity_value, volume_id, file_id, display_name,
                 placement_kind, monitor_id, x, y, pane_id, grid_column, grid_row
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                item.identity().persistent_key(),
                identity_kind,
                identity_value,
                volume_id,
                file_id,
                item.display_name(),
                placement_kind,
                monitor_id,
                x,
                y,
                pane_id,
                grid_column,
                grid_row,
            ],
        )?;
    }
    Ok(())
}

struct PersistedDesktopItem {
    identity_kind: String,
    identity_value: String,
    volume_id: Option<String>,
    file_id: Option<String>,
    display_name: String,
    placement_kind: String,
    monitor_id: Option<String>,
    x: Option<f32>,
    y: Option<f32>,
    pane_id: Option<i64>,
    grid_column: Option<i64>,
    grid_row: Option<i64>,
}

impl PersistedDesktopItem {
    fn into_item(self) -> Result<DesktopItem, StoreError> {
        let identity = match self.identity_kind.as_str() {
            "filesystem" => ShellIdentity::FileSystem {
                path: PathBuf::from(self.identity_value),
                volume_id: parse_optional(self.volume_id.as_deref(), "volume id")?,
                file_id: parse_optional(self.file_id.as_deref(), "file id")?,
            },
            "namespace" => ShellIdentity::Namespace {
                parsing_name: self.identity_value,
            },
            kind => {
                return Err(StoreError::InvalidData(format!(
                    "unknown desktop identity {kind}"
                )));
            }
        };
        let placement = match self.placement_kind.as_str() {
            "free" => DesktopPlacement::FreeDesktop {
                monitor: MonitorId::new(required(self.monitor_id, "monitor id")?),
                position: PointDip::new(
                    required(self.x, "desktop x")?,
                    required(self.y, "desktop y")?,
                ),
            },
            "pane" => DesktopPlacement::Pane {
                pane_id: PanelId::new(
                    u64::try_from(required(self.pane_id, "pane id")?)
                        .map_err(|_| StoreError::InvalidData("pane id is negative".into()))?,
                ),
                position: GridPosition::new(
                    u32::try_from(required(self.grid_column, "grid column")?).map_err(|_| {
                        StoreError::InvalidData("grid column is outside u32 range".into())
                    })?,
                    u32::try_from(required(self.grid_row, "grid row")?).map_err(|_| {
                        StoreError::InvalidData("grid row is outside u32 range".into())
                    })?,
                ),
            },
            kind => {
                return Err(StoreError::InvalidData(format!(
                    "unknown desktop placement {kind}"
                )));
            }
        };
        let mut item = DesktopItem::new(identity, self.display_name);
        item.set_placement(placement);
        Ok(item)
    }
}

fn parse_optional<T: std::str::FromStr>(
    value: Option<&str>,
    label: &str,
) -> Result<Option<T>, StoreError> {
    value
        .map(|value| {
            value
                .parse()
                .map_err(|_| StoreError::InvalidData(format!("invalid {label}")))
        })
        .transpose()
}

fn required<T>(value: Option<T>, label: &str) -> Result<T, StoreError> {
    value.ok_or_else(|| StoreError::InvalidData(format!("missing {label}")))
}

fn encode_backdrop(backdrop: Backdrop) -> (&'static str, Option<f32>, Option<u32>) {
    match backdrop {
        Backdrop::Tuned { material, strength } => (
            match material {
                desktop_core::MaterialKind::Acrylic => "acrylic_tuned",
                desktop_core::MaterialKind::Mica => "mica_tuned",
            },
            Some(f32::from(strength) / 100.0),
            None,
        ),
        Backdrop::Mica => ("mica", None, None),
        Backdrop::MicaAlt => ("mica_alt", None, None),
        Backdrop::Acrylic => ("acrylic", None, None),
        Backdrop::Translucent { opacity } => ("translucent", Some(opacity), None),
        Backdrop::Solid { color, opacity } => ("solid", Some(opacity), Some(color)),
    }
}

fn decode_backdrop(
    kind: &str,
    opacity: Option<f32>,
    color: Option<u32>,
) -> Result<Backdrop, StoreError> {
    let opacity = opacity.unwrap_or(0.86);
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(StoreError::InvalidData("invalid backdrop opacity".into()));
    }
    Ok(match kind {
        "acrylic_tuned" => Backdrop::Acrylic.with_strength((opacity * 100.0).round() as u8),
        "mica_tuned" => Backdrop::Mica.with_strength((opacity * 100.0).round() as u8),
        "mica_alt_tuned" => Backdrop::MicaAlt, // Normalize legacy adjustable Alt to the fixed preset.
        "mica" => Backdrop::Mica,
        "mica_alt" => Backdrop::MicaAlt,
        "acrylic" => Backdrop::Acrylic,
        "translucent" => Backdrop::Translucent { opacity },
        "solid" => {
            let color = color
                .filter(|v| *v <= 0xffffff)
                .ok_or_else(|| StoreError::InvalidData("invalid solid color".into()))?;
            Backdrop::Solid { color, opacity }
        }
        _ => return Err(StoreError::InvalidData(format!("unknown backdrop {kind}"))),
    })
}

struct PersistedPanel {
    id: i64,
    title: String,
    rect: RectDip,
    collapsed: bool,
    locked: bool,
    backdrop_kind: String,
    opacity: Option<f32>,
    color: Option<u32>,
}

impl PersistedPanel {
    fn into_panel(self) -> Result<Panel, StoreError> {
        let id = u64::try_from(self.id)
            .map_err(|_| StoreError::InvalidData("panel id is negative".into()))?;
        let backdrop = decode_backdrop(&self.backdrop_kind, self.opacity, self.color)?;

        let mut panel = Panel::new(PanelId::new(id), self.title, self.rect);
        panel.set_collapsed(self.collapsed);
        panel.set_locked(self.locked);
        panel.set_backdrop(backdrop);
        Ok(panel)
    }
}

#[derive(Debug)]
pub enum StoreError {
    Database(rusqlite::Error),
    InvalidData(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "database error: {error}"),
            Self::InvalidData(error) => write!(formatter, "invalid workspace data: {error}"),
        }
    }
}

impl Error for StoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::InvalidData(_) => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unchanged_preferences_do_not_count_as_database_changes() {
        let store = WorkspaceStore::open_in_memory().unwrap();
        store.save_preference("test", "first").unwrap();
        let changes = store.change_count();
        store.save_preference("test", "first").unwrap();
        assert_eq!(store.change_count(), changes);
        store.save_preference("test", "second").unwrap();
        assert_eq!(store.change_count(), changes + 1);
        assert_eq!(store.preference("test").unwrap().as_deref(), Some("second"));
        let options = Workspace::new().pane_options();
        store.save_pane_options(options).unwrap();
        let changes = store.change_count();
        store.save_pane_options(options).unwrap();
        assert_eq!(store.change_count(), changes);
    }

    #[test]
    fn material_strength_round_trips_and_remembers_each_material() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let mut workspace = Workspace::new();
        workspace
            .add_panel(Panel::new(
                PanelId::new(1),
                "Material",
                RectDip::new(0.0, 0.0, 300.0, 200.0),
            ))
            .unwrap();
        for base in [Backdrop::Acrylic, Backdrop::Mica] {
            for strength in [0, 33, 50, 100] {
                let value = base.with_strength(strength);
                workspace.set_appearance(desktop_core::PanelTheme::Dark, value);
                store.save_workspace(&workspace).unwrap();
                let restored = store.load_workspace().unwrap();
                assert_eq!(restored.appearance(), workspace.appearance());
                assert_eq!(restored.panels()[0].backdrop(), value);
            }
        }
        for base in [Backdrop::Acrylic, Backdrop::Mica] {
            assert_eq!(
                store
                    .preference(base.strength_key().unwrap())
                    .unwrap()
                    .as_deref(),
                Some("100")
            );
        }
        workspace.set_appearance(
            desktop_core::PanelTheme::Dark,
            Backdrop::Tuned {
                material: desktop_core::MaterialKind::Mica,
                strength: 200,
            },
        );
        assert!(store.save_workspace(&workspace).is_err());
        assert_eq!(
            store.load_workspace().unwrap().appearance().unwrap().1,
            Backdrop::Mica.with_strength(100)
        );
        for opacity in [0.0, 0.5, 1.0] {
            assert_eq!(
                super::decode_backdrop("mica_alt_tuned", Some(opacity), None).unwrap(),
                Backdrop::MicaAlt
            );
        }
        assert_eq!(Backdrop::MicaAlt.strength(), None);
        assert_eq!(Backdrop::MicaAlt.strength_key(), None);
        assert!(super::decode_backdrop("mica_tuned", Some(f32::NAN), None).is_err());
    }

    #[test]
    fn solid_style_round_trips_and_survives_switching_material() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let mut workspace = Workspace::new();
        workspace
            .add_panel(Panel::new(
                PanelId::new(1),
                "Solid",
                RectDip::new(0.0, 0.0, 300.0, 200.0),
            ))
            .unwrap();
        for opacity in [0.0, 0.5, 1.0] {
            let solid = Backdrop::Solid {
                color: 0x1234ab,
                opacity,
            };
            workspace.set_appearance(desktop_core::PanelTheme::Dark, solid);
            store.save_workspace(&workspace).unwrap();
            let restored = store.load_workspace().unwrap();
            assert_eq!(restored.appearance(), workspace.appearance());
            assert_eq!(restored.panels()[0].backdrop(), solid);
        }
        workspace.set_appearance(desktop_core::PanelTheme::Dark, Backdrop::Mica);
        store.save_workspace(&workspace).unwrap();
        assert_eq!(
            store.preference("solid_style").unwrap().as_deref(),
            Some("1193131|1")
        );
        for opacity in [f32::NAN, -0.1, 1.1] {
            workspace.set_appearance(
                desktop_core::PanelTheme::Dark,
                Backdrop::Solid {
                    color: 0x1234ab,
                    opacity,
                },
            );
            assert!(store.save_workspace(&workspace).is_err());
            assert_eq!(
                store.load_workspace().unwrap().appearance().unwrap().1,
                Backdrop::Mica
            );
        }
    }

    #[test]
    fn v10_color_migration_and_old_backup_preserve_layout() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let mut workspace = Workspace::new();
        workspace
            .add_panel(Panel::new(
                PanelId::new(3),
                "Before",
                RectDip::new(12.0, 24.0, 360.0, 240.0),
            ))
            .unwrap();
        store.save_workspace(&workspace).unwrap();
        store.connection.execute_batch("ALTER TABLE panels DROP COLUMN color; UPDATE metadata SET value='10' WHERE key='schema_version';").unwrap();
        let restored = WorkspaceStore::from_connection(store.connection).unwrap();
        assert_eq!(
            restored.load_workspace().unwrap().panels(),
            workspace.panels()
        );
        assert_eq!(
            restored.preference("schema_version").unwrap().as_deref(),
            Some("11")
        );
    }

    #[test]
    fn search_panes_round_trip_and_do_not_retain_folder_sources() {
        use desktop_core::{Panel, PanelId, RectDip, Workspace};
        let mut store = super::WorkspaceStore::open_in_memory().unwrap();
        let mut panel = Panel::new(
            PanelId::new(7),
            "Everything",
            RectDip::new(120.0, 140.0, 860.0, 520.0),
        );
        panel.set_folder(Some(std::path::PathBuf::from(r"C:\folder")));
        panel.set_search(true);
        assert!(panel.folder().is_none());
        let mut workspace = Workspace::from_panels(vec![panel]).unwrap();
        store.save_workspace(&workspace).unwrap();
        let loaded = store.load_workspace().unwrap();
        assert_eq!(loaded.panels(), workspace.panels());
        assert!(loaded.desktop_items().is_empty());
        workspace
            .panel_mut(PanelId::new(7))
            .unwrap()
            .set_folder(Some(std::path::PathBuf::from(r"C:\folder")));
        assert!(!workspace.panel(PanelId::new(7)).unwrap().is_search());
        store.save_workspace(&workspace).unwrap();
        assert!(store.preference("panel_search:7").unwrap().is_none());
        workspace
            .panel_mut(PanelId::new(7))
            .unwrap()
            .set_search(true);
        store.save_workspace(&workspace).unwrap();
        workspace.remove_panel(PanelId::new(7));
        store.save_workspace(&workspace).unwrap();
        assert!(store.preference("panel_search:7").unwrap().is_none());
    }
    #[test]
    fn folder_sources_survive_reopen_and_are_removed_with_the_panel() {
        let mut store = super::WorkspaceStore::open_in_memory().unwrap();
        let mut folder = desktop_core::Panel::new(
            desktop_core::PanelId::new(2),
            "映射",
            desktop_core::RectDip::default(),
        );
        let path = std::path::PathBuf::from(r"C:\资料\尚未挂载");
        folder.set_folder(Some(path.clone()));
        let mut workspace = desktop_core::Workspace::from_panels(vec![
            desktop_core::Panel::new(
                desktop_core::PanelId::new(1),
                "桌面",
                desktop_core::RectDip::default(),
            ),
            folder,
        ])
        .unwrap();
        store.save_preference("peek", "keep").unwrap();
        store.save_workspace(&workspace).unwrap();
        let loaded = store.load_workspace().unwrap();
        assert_eq!(
            loaded
                .panel(desktop_core::PanelId::new(2))
                .unwrap()
                .folder(),
            Some(path.as_path())
        );
        assert!(
            loaded
                .panel(desktop_core::PanelId::new(1))
                .unwrap()
                .folder()
                .is_none()
        );
        assert!(loaded.desktop_items().is_empty());
        assert!(
            loaded
                .panel(desktop_core::PanelId::new(2))
                .unwrap()
                .folder_list()
        );
        workspace
            .panel_mut(desktop_core::PanelId::new(2))
            .unwrap()
            .set_folder_list(false);
        store.save_workspace(&workspace).unwrap();
        assert!(
            !store
                .load_workspace()
                .unwrap()
                .panel(desktop_core::PanelId::new(2))
                .unwrap()
                .folder_list()
        );
        workspace.remove_panel(desktop_core::PanelId::new(2));
        store.save_workspace(&workspace).unwrap();
        assert!(store.preference("panel_folder:2").unwrap().is_none());
        assert!(store.preference("panel_folder_view:2").unwrap().is_none());
        assert_eq!(store.preference("peek").unwrap().as_deref(), Some("keep"));
    }
    use super::{SCHEMA_VERSION, WorkspaceStore};
    use desktop_core::{
        Backdrop, DesktopItem, DesktopPlacement, GridPosition, Panel, PanelId, RectDip,
        ShellIdentity, Workspace,
    };
    use rusqlite::Connection;

    #[test]
    fn workspace_round_trips() {
        let mut panel = Panel::new(
            PanelId::new(42),
            "Downloads",
            RectDip::new(-300.0, 75.0, 540.0, 480.0),
        );
        panel.set_backdrop(Backdrop::Translucent { opacity: 0.72 });
        panel.set_collapsed(true);
        panel.set_auto_hide(true);
        panel.set_always_on_top(true);
        panel.set_theme(desktop_core::PanelTheme::Light);

        let mut workspace = Workspace::from_panels(vec![panel]).unwrap();
        let mut desktop_item = DesktopItem::new(
            ShellIdentity::Namespace {
                parsing_name: "::{645FF040-5081-101B-9F08-00AA002F954E}".into(),
            },
            "Recycle Bin",
        );
        desktop_item.set_placement(DesktopPlacement::Pane {
            pane_id: PanelId::new(42),
            position: GridPosition::new(1, 2),
        });
        workspace.reconcile_desktop_items([desktop_item]);
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        store.save_workspace(&workspace).unwrap();
        let loaded = store.load_workspace().unwrap();

        assert_eq!(loaded, workspace);
    }

    #[test]
    fn theme_choices_round_trip() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let panel = Panel::new(PanelId::new(1), "Pane", RectDip::default());
        let mut workspace = Workspace::from_panels(vec![panel]).unwrap();
        for theme in [
            desktop_core::PanelTheme::System,
            desktop_core::PanelTheme::Light,
            desktop_core::PanelTheme::Dark,
        ] {
            workspace
                .panel_mut(PanelId::new(1))
                .unwrap()
                .set_theme(theme);
            workspace
                .panel_mut(PanelId::new(1))
                .unwrap()
                .set_backdrop(Backdrop::MicaAlt);
            store.save_workspace(&workspace).unwrap();
            assert_eq!(store.load_workspace().unwrap(), workspace);
        }
    }

    #[test]
    fn global_appearance_survives_empty_workspace_and_new_panels() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let mut workspace = Workspace::new();
        workspace.set_appearance(desktop_core::PanelTheme::Dark, Backdrop::MicaAlt);
        store.save_workspace(&workspace).unwrap();
        let mut restored = store.load_workspace().unwrap();
        assert_eq!(restored.appearance(), workspace.appearance());
        restored
            .add_panel(Panel::new(PanelId::new(1), "New", RectDip::default()))
            .unwrap();
        assert_eq!(restored.panels()[0].theme(), desktop_core::PanelTheme::Dark);
        assert_eq!(restored.panels()[0].backdrop(), Backdrop::MicaAlt);
        store.save_workspace(&restored).unwrap();
        assert_eq!(store.load_workspace().unwrap(), restored);
    }

    #[test]
    fn save_replaces_previous_snapshot() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let first =
            Workspace::from_panels(vec![Panel::new(PanelId::new(1), "One", RectDip::default())])
                .unwrap();
        store.save_workspace(&first).unwrap();
        store.save_workspace(&Workspace::new()).unwrap();
        assert!(store.load_workspace().unwrap().panels().is_empty());
    }

    #[test]
    fn rejects_other_schema_versions_without_modifying_data() {
        for version in ["1", "7", "12"] {
            let connection = Connection::open_in_memory().unwrap();
            connection
                .execute_batch("CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);")
                .unwrap();
            connection
                .execute(
                    "INSERT INTO metadata VALUES ('schema_version', ?1)",
                    [version],
                )
                .unwrap();
            assert!(super::initialize_schema(&connection).is_err());
            let actual: String = connection
                .query_row(
                    "SELECT value FROM metadata WHERE key = 'schema_version'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(actual, version);
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }
    }

    #[test]
    fn option_update_does_not_rewrite_workspace_rows() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let workspace = Workspace::from_panels(vec![Panel::new(
            PanelId::new(1),
            "Keep",
            RectDip::default(),
        )])
        .unwrap();
        store.save_workspace(&workspace).unwrap();
        store.connection.execute_batch("CREATE TRIGGER forbid_panel_delete BEFORE DELETE ON panels BEGIN SELECT RAISE(ABORT, 'unexpected workspace rewrite'); END;").unwrap();
        let options = desktop_core::PaneOptions {
            corner_radius: 24.0,
            ..desktop_core::PaneOptions::DEFAULT
        };
        store.save_pane_options(options).unwrap();
        let restored = store.load_workspace().unwrap();
        assert_eq!(restored.panels(), workspace.panels());
        assert_eq!(restored.pane_options(), options);
    }

    #[test]
    fn pane_options_round_trip_without_panels() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        assert_eq!(
            store.load_workspace().unwrap().pane_options(),
            desktop_core::PaneOptions::DEFAULT
        );
        for bits in 0..8 {
            let options = desktop_core::PaneOptions {
                corner_radius: if bits & 1 != 0 { 24.0 } else { 0.0 },
                border: bits & 2 != 0,
                snap: bits & 4 != 0,
                text: desktop_core::PanelText::Auto,
                text_protection: true,
            };
            let mut workspace = Workspace::new();
            workspace.set_pane_options(options);
            store.save_workspace(&workspace).unwrap();
            let reopened = WorkspaceStore::from_connection(store.connection).unwrap();
            assert_eq!(reopened.load_workspace().unwrap().pane_options(), options);
            store = reopened;
        }
    }

    #[test]
    fn panel_text_modes_round_trip_and_legacy_options_default_to_auto() {
        use desktop_core::PanelText;
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        store
            .save_preference("pane_options", "6|true|false")
            .unwrap();
        assert_eq!(
            store.load_workspace().unwrap().pane_options().text,
            PanelText::Auto
        );
        for value in ["6|true|false", "6|true|false|dark"] {
            store.save_preference("pane_options", value).unwrap();
            assert!(
                !store
                    .load_workspace()
                    .unwrap()
                    .pane_options()
                    .text_protection
            );
        }
        for (text, text_protection) in [PanelText::Auto, PanelText::Light, PanelText::Dark]
            .into_iter()
            .flat_map(|text| [true, false].map(|enabled| (text, enabled)))
        {
            let options = desktop_core::PaneOptions {
                text,
                text_protection,
                ..Default::default()
            };
            store.save_pane_options(options).unwrap();
            let workspace = store.load_workspace().unwrap();
            assert_eq!(workspace.pane_options(), options);
            store.save_workspace(&workspace).unwrap();
            assert_eq!(store.load_workspace().unwrap().pane_options(), options);
        }
    }

    #[test]
    fn fractional_corner_radius_round_trips_and_rejects_non_finite_values() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let options = desktop_core::PaneOptions {
            corner_radius: 6.375,
            ..Default::default()
        };
        store.save_pane_options(options).unwrap();
        let workspace = store.load_workspace().unwrap();
        assert_eq!(workspace.pane_options().corner_radius, 6.375);
        store.save_workspace(&workspace).unwrap();
        assert_eq!(
            store.load_workspace().unwrap().pane_options().corner_radius,
            6.375
        );
        for radius in ["NaN", "inf", "-0.5", "24.1"] {
            store
                .save_preference("pane_options", &format!("{radius}|true|true"))
                .unwrap();
            assert!(store.load_workspace().is_err());
        }
    }

    #[test]
    fn current_database_reopens_without_reinitializing_it() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let workspace = Workspace::from_panels(vec![Panel::new(
            PanelId::new(1),
            "Current",
            RectDip::default(),
        )])
        .unwrap();
        store.save_workspace(&workspace).unwrap();
        let version: String = store
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION.to_string());
        let reopened = WorkspaceStore::from_connection(store.connection).unwrap();
        assert_eq!(reopened.load_workspace().unwrap(), workspace);
    }
}
