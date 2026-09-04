use desktop_core::{
    Backdrop, DesktopItem, DesktopPlacement, GridPosition, MonitorId, Panel, PanelIcon, PanelId,
    PanelSource, PointDip, RectDip, ShellIdentity, Workspace,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: i64 = 5;

pub struct WorkspaceStore {
    connection: Connection,
}

impl WorkspaceStore {
    /// Opens or creates a `LucidPane` workspace database.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be opened or migrated.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
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
        migrate(&connection)?;
        Ok(Self { connection })
    }

    /// Loads the complete workspace.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid persisted data or a database failure.
    pub fn load_workspace(&self) -> Result<Workspace, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id, title, source_kind, source_value, x, y, width, height, \
             collapsed, locked, backdrop_kind, opacity, icon_path FROM panels ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            let raw_id: i64 = row.get(0)?;
            let source_kind: String = row.get(2)?;
            let source_value: Option<String> = row.get(3)?;
            let backdrop_kind: String = row.get(10)?;
            let opacity: Option<f32> = row.get(11)?;
            Ok(PersistedPanel {
                id: raw_id,
                title: row.get(1)?,
                source_kind,
                source_value,
                rect: RectDip::new(row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?),
                collapsed: row.get(8)?,
                locked: row.get(9)?,
                backdrop_kind,
                opacity,
                icon_path: row.get(12)?,
            })
        })?;

        let mut panels = Vec::new();
        for row in rows {
            panels.push(row?.into_panel()?);
        }
        drop(statement);
        for panel in &mut panels {
            let panel_id = i64::try_from(panel.id().get())
                .map_err(|_| StoreError::InvalidData("panel id exceeds SQLite range".into()))?;
            let mut item_statement = self
                .connection
                .prepare("SELECT path FROM panel_items WHERE panel_id = ?1 ORDER BY item_order")?;
            let item_rows = item_statement.query_map([panel_id], |row| row.get::<_, String>(0))?;
            for item_path in item_rows {
                panel.add_item(PathBuf::from(item_path?));
            }
        }
        let mut workspace = Workspace::from_panels(panels)
            .map_err(|error| StoreError::InvalidData(error.to_string()))?;
        workspace.reconcile_desktop_items(load_desktop_items(&self.connection)?);
        Ok(workspace)
    }

    /// Replaces the persisted workspace in one transaction.
    ///
    /// # Errors
    ///
    /// Returns an error when any panel cannot be serialized or committed.
    pub fn save_workspace(&mut self, workspace: &Workspace) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM desktop_items", [])?;
        transaction.execute("DELETE FROM panels", [])?;
        for panel in workspace.panels() {
            insert_panel(&transaction, panel)?;
            insert_panel_items(&transaction, panel)?;
        }
        insert_desktop_items(&transaction, workspace.desktop_items())?;
        transaction.commit()?;
        Ok(())
    }
}

fn migrate(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS metadata (
             key TEXT PRIMARY KEY NOT NULL,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS panels (
             id INTEGER PRIMARY KEY NOT NULL,
             title TEXT NOT NULL,
             source_kind TEXT NOT NULL,
             source_value TEXT,
             x REAL NOT NULL,
             y REAL NOT NULL,
             width REAL NOT NULL,
             height REAL NOT NULL,
             collapsed INTEGER NOT NULL,
             locked INTEGER NOT NULL,
             backdrop_kind TEXT NOT NULL,
             opacity REAL,
             icon_path TEXT
         );
         CREATE TABLE IF NOT EXISTS panel_items (
             panel_id INTEGER NOT NULL REFERENCES panels(id) ON DELETE CASCADE,
             item_order INTEGER NOT NULL,
             path TEXT NOT NULL,
             PRIMARY KEY(panel_id, item_order)
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

    let version: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    match version.as_deref() {
        None => {
            connection.execute(
                "INSERT INTO metadata(key, value) VALUES ('schema_version', ?1)",
                [SCHEMA_VERSION.to_string()],
            )?;
        }
        Some("1") => {
            connection.execute_batch(
                "ALTER TABLE panels ADD COLUMN icon_path TEXT;
                 UPDATE panels SET source_kind = 'manual', source_value = CAST(id AS TEXT)
                 WHERE source_kind = 'folder';
                 UPDATE panels SET x = 120, y = 120, width = 420, height = 360
                 WHERE x < 32 OR y < 32 OR width > 800 OR height > 720;
                 UPDATE metadata SET value = '5' WHERE key = 'schema_version';",
            )?;
        }
        Some("2") => {
            connection.execute_batch(
                "UPDATE panels SET source_kind = 'manual', source_value = CAST(id AS TEXT)
                 WHERE source_kind = 'folder';
                 UPDATE panels SET x = 120, y = 120, width = 420, height = 360
                 WHERE x < 32 OR y < 32 OR width > 800 OR height > 720;
                 UPDATE metadata SET value = '5' WHERE key = 'schema_version';",
            )?;
        }
        Some("3") => {
            connection.execute_batch(
                "UPDATE panels SET x = 120, y = 120, width = 420, height = 360
                 WHERE x < 32 OR y < 32 OR width > 800 OR height > 720;
                 UPDATE metadata SET value = '5' WHERE key = 'schema_version';",
            )?;
        }
        Some("4") => {
            connection.execute(
                "UPDATE metadata SET value = '5' WHERE key = 'schema_version'",
                [],
            )?;
        }
        Some("5") => {}
        Some(value) => {
            return Err(StoreError::InvalidData(format!(
                "unsupported schema version {value}"
            )));
        }
    }
    Ok(())
}

fn insert_panel(transaction: &Transaction<'_>, panel: &Panel) -> Result<(), StoreError> {
    let id = i64::try_from(panel.id().get())
        .map_err(|_| StoreError::InvalidData("panel id exceeds SQLite range".into()))?;
    let (source_kind, source_value) = encode_source(panel.source());
    let (backdrop_kind, opacity) = encode_backdrop(panel.backdrop());
    let icon_path = match panel.icon() {
        PanelIcon::Automatic => None,
        PanelIcon::Custom(path) => Some(path.as_os_str().to_string_lossy().into_owned()),
    };
    let rect = panel.rect();
    transaction.execute(
        "INSERT INTO panels(
             id, title, source_kind, source_value, x, y, width, height,
             collapsed, locked, backdrop_kind, opacity, icon_path
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            id,
            panel.title(),
            source_kind,
            source_value,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            panel.collapsed(),
            panel.locked(),
            backdrop_kind,
            opacity,
            icon_path,
        ],
    )?;
    Ok(())
}

fn insert_panel_items(transaction: &Transaction<'_>, panel: &Panel) -> Result<(), StoreError> {
    let panel_id = i64::try_from(panel.id().get())
        .map_err(|_| StoreError::InvalidData("panel id exceeds SQLite range".into()))?;
    for (item_order, path) in panel.item_paths().iter().enumerate() {
        let item_order = i64::try_from(item_order)
            .map_err(|_| StoreError::InvalidData("panel contains too many items".into()))?;
        transaction.execute(
            "INSERT INTO panel_items(panel_id, item_order, path) VALUES (?1, ?2, ?3)",
            params![
                panel_id,
                item_order,
                path.as_os_str().to_string_lossy().into_owned()
            ],
        )?;
    }
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

fn encode_source(source: &PanelSource) -> (&'static str, Option<String>) {
    match source {
        PanelSource::Folder { path } => (
            "folder",
            Some(path.as_os_str().to_string_lossy().into_owned()),
        ),
        PanelSource::DesktopCollection => ("desktop", None),
        PanelSource::ManualCollection { collection_id } => {
            ("manual", Some(collection_id.to_string()))
        }
    }
}

fn encode_backdrop(backdrop: Backdrop) -> (&'static str, Option<f32>) {
    match backdrop {
        Backdrop::Mica => ("mica", None),
        Backdrop::MicaAlt => ("mica_alt", None),
        Backdrop::Acrylic => ("acrylic", None),
        Backdrop::Translucent { opacity } => ("translucent", Some(opacity)),
    }
}

struct PersistedPanel {
    id: i64,
    title: String,
    source_kind: String,
    source_value: Option<String>,
    rect: RectDip,
    collapsed: bool,
    locked: bool,
    backdrop_kind: String,
    opacity: Option<f32>,
    icon_path: Option<String>,
}

impl PersistedPanel {
    fn into_panel(self) -> Result<Panel, StoreError> {
        let id = u64::try_from(self.id)
            .map_err(|_| StoreError::InvalidData("panel id is negative".into()))?;
        let source = match self.source_kind.as_str() {
            "folder" => PanelSource::Folder {
                path: self
                    .source_value
                    .ok_or_else(|| StoreError::InvalidData("folder path is missing".into()))?
                    .into(),
            },
            "desktop" => PanelSource::DesktopCollection,
            "manual" => PanelSource::ManualCollection {
                collection_id: self
                    .source_value
                    .ok_or_else(|| StoreError::InvalidData("collection id is missing".into()))?
                    .parse()
                    .map_err(|_| StoreError::InvalidData("collection id is invalid".into()))?,
            },
            kind => return Err(StoreError::InvalidData(format!("unknown source {kind}"))),
        };
        let backdrop = match self.backdrop_kind.as_str() {
            "mica" => Backdrop::Mica,
            "mica_alt" => Backdrop::MicaAlt,
            "acrylic" => Backdrop::Acrylic,
            "translucent" => Backdrop::Translucent {
                opacity: self.opacity.unwrap_or(0.86),
            },
            kind => return Err(StoreError::InvalidData(format!("unknown backdrop {kind}"))),
        };

        let mut panel = Panel::new(PanelId::new(id), self.title, source, self.rect);
        panel.set_collapsed(self.collapsed);
        panel.set_locked(self.locked);
        panel.set_backdrop(backdrop);
        if let Some(path) = self.icon_path {
            panel.set_icon(PanelIcon::Custom(path.into()));
        }
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
    use super::WorkspaceStore;
    use desktop_core::{
        Backdrop, DesktopItem, DesktopPlacement, GridPosition, Panel, PanelIcon, PanelId,
        PanelSource, RectDip, ShellIdentity, Workspace,
    };
    use rusqlite::Connection;
    use std::path::PathBuf;

    #[test]
    fn workspace_round_trips() {
        let mut panel = Panel::new(
            PanelId::new(42),
            "Downloads",
            PanelSource::Folder {
                path: PathBuf::from(r"D:\Downloads"),
            },
            RectDip::new(-300.0, 75.0, 540.0, 480.0),
        );
        panel.set_backdrop(Backdrop::Translucent { opacity: 0.72 });
        panel.set_collapsed(true);
        panel.set_icon(PanelIcon::Custom(PathBuf::from(r"D:\Icons\downloads.ico")));
        panel.add_item(PathBuf::from(r"C:\Users\Test\Desktop\Editor.lnk"));
        panel.add_item(PathBuf::from(r"C:\Users\Test\Desktop\Notes.txt"));

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
    fn save_replaces_previous_snapshot() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let first = Workspace::from_panels(vec![Panel::new(
            PanelId::new(1),
            "One",
            PanelSource::DesktopCollection,
            RectDip::default(),
        )])
        .unwrap();
        store.save_workspace(&first).unwrap();
        store.save_workspace(&Workspace::new()).unwrap();
        assert!(store.load_workspace().unwrap().panels().is_empty());
    }

    #[test]
    fn version_one_database_migrates_with_automatic_icons() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE metadata (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 INSERT INTO metadata(key, value) VALUES ('schema_version', '1');
                 CREATE TABLE panels (
                     id INTEGER PRIMARY KEY NOT NULL,
                     title TEXT NOT NULL,
                     source_kind TEXT NOT NULL,
                     source_value TEXT,
                     x REAL NOT NULL,
                     y REAL NOT NULL,
                     width REAL NOT NULL,
                     height REAL NOT NULL,
                     collapsed INTEGER NOT NULL,
                     locked INTEGER NOT NULL,
                     backdrop_kind TEXT NOT NULL,
                     opacity REAL
                 );
                 INSERT INTO panels VALUES (
                     1, 'Desktop', 'desktop', NULL, 10, 10, 420, 360, 0, 0, 'mica', NULL
                 );",
            )
            .unwrap();

        let store = WorkspaceStore::from_connection(connection).unwrap();
        let workspace = store.load_workspace().unwrap();
        assert_eq!(workspace.panels()[0].icon(), &PanelIcon::Automatic);
    }

    #[test]
    fn version_two_folder_portal_migrates_to_an_empty_manual_pane() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE metadata (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 INSERT INTO metadata(key, value) VALUES ('schema_version', '2');
                 CREATE TABLE panels (
                     id INTEGER PRIMARY KEY NOT NULL,
                     title TEXT NOT NULL,
                     source_kind TEXT NOT NULL,
                     source_value TEXT,
                     x REAL NOT NULL,
                     y REAL NOT NULL,
                     width REAL NOT NULL,
                     height REAL NOT NULL,
                     collapsed INTEGER NOT NULL,
                     locked INTEGER NOT NULL,
                     backdrop_kind TEXT NOT NULL,
                     opacity REAL,
                     icon_path TEXT
                 );
                 INSERT INTO panels VALUES (
                     1, 'Desktop', 'folder', 'C:\\Users\\Test\\Desktop',
                     10, 10, 420, 360, 0, 0, 'mica', NULL, NULL
                 );",
            )
            .unwrap();

        let store = WorkspaceStore::from_connection(connection).unwrap();
        let workspace = store.load_workspace().unwrap();
        assert_eq!(
            workspace.panels()[0].source(),
            &PanelSource::ManualCollection { collection_id: 1 }
        );
        assert!(workspace.panels()[0].item_paths().is_empty());
    }

    #[test]
    fn version_three_repairs_legacy_offscreen_geometry() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE metadata (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 INSERT INTO metadata(key, value) VALUES ('schema_version', '3');
                 CREATE TABLE panels (
                     id INTEGER PRIMARY KEY NOT NULL,
                     title TEXT NOT NULL,
                     source_kind TEXT NOT NULL,
                     source_value TEXT,
                     x REAL NOT NULL,
                     y REAL NOT NULL,
                     width REAL NOT NULL,
                     height REAL NOT NULL,
                     collapsed INTEGER NOT NULL,
                     locked INTEGER NOT NULL,
                     backdrop_kind TEXT NOT NULL,
                     opacity REAL,
                     icon_path TEXT
                 );
                 CREATE TABLE panel_items (
                     panel_id INTEGER NOT NULL REFERENCES panels(id) ON DELETE CASCADE,
                     item_order INTEGER NOT NULL,
                     path TEXT NOT NULL,
                     PRIMARY KEY(panel_id, item_order)
                 );
                 INSERT INTO panels VALUES (
                     1, 'Project', 'manual', '1', 0, 0, 460, 836, 0, 0, 'mica', NULL, NULL
                 );",
            )
            .unwrap();

        let store = WorkspaceStore::from_connection(connection).unwrap();
        let rect = store.load_workspace().unwrap().panels()[0].rect();
        assert_eq!(rect, RectDip::default());
    }

    #[test]
    fn version_four_adds_desktop_identity_and_placement_storage() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE metadata (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 INSERT INTO metadata(key, value) VALUES ('schema_version', '4');
                 CREATE TABLE panels (
                     id INTEGER PRIMARY KEY NOT NULL,
                     title TEXT NOT NULL,
                     source_kind TEXT NOT NULL,
                     source_value TEXT,
                     x REAL NOT NULL,
                     y REAL NOT NULL,
                     width REAL NOT NULL,
                     height REAL NOT NULL,
                     collapsed INTEGER NOT NULL,
                     locked INTEGER NOT NULL,
                     backdrop_kind TEXT NOT NULL,
                     opacity REAL,
                     icon_path TEXT
                 );
                 CREATE TABLE panel_items (
                     panel_id INTEGER NOT NULL REFERENCES panels(id) ON DELETE CASCADE,
                     item_order INTEGER NOT NULL,
                     path TEXT NOT NULL,
                     PRIMARY KEY(panel_id, item_order)
                 );",
            )
            .unwrap();

        let store = WorkspaceStore::from_connection(connection).unwrap();
        let workspace = store.load_workspace().unwrap();
        assert!(workspace.desktop_items().is_empty());
        let version: String = store
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "5");
    }
}
