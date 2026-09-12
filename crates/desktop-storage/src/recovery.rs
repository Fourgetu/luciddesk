use super::*;

pub(super) fn migrate_v8(tx: &Transaction<'_>) -> Result<(), StoreError> {
    // v8 desktop identities already use the current representation. Legacy manual
    // path lists have no equivalent in the desktop-only membership model.
    let manual_items: i64 = tx.query_row("SELECT count(*) FROM panel_items", [], |r| r.get(0))?;
    let invalid_sources: i64 = tx.query_row(
        "SELECT count(*) FROM panels WHERE source_kind NOT IN ('desktop','manual','folder') OR (source_kind='folder' AND (source_value IS NULL OR source_value=''))",
        [], |r| r.get(0))?;
    if manual_items != 0 || invalid_sources != 0 {
        return Err(StoreError::InvalidData("v8 配置包含无法自动转换的旧版路径集合或来源类型，原配置已保留，请先使用旧版导出这些项目".into()));
    }
    tx.execute_batch(
        "INSERT OR REPLACE INTO metadata(key,value)
         SELECT 'panel_folder:' || id, source_value FROM panels WHERE source_kind='folder';
         ALTER TABLE panels DROP COLUMN source_kind;
         ALTER TABLE panels DROP COLUMN source_value;
         ALTER TABLE panels DROP COLUMN icon_path;
         DROP TABLE panel_items;",
    )?;
    Ok(())
}

pub(super) const LAYOUT_SCHEMA: &str = "CREATE TABLE monitor_layouts (
    topology TEXT NOT NULL, panel_id INTEGER NOT NULL,
    x REAL NOT NULL, y REAL NOT NULL, width REAL NOT NULL, height REAL NOT NULL,
    PRIMARY KEY(topology, panel_id))";

pub(super) fn unique_backup_path(path: &Path, label: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.with_extension(format!("{label}-{stamp}.db"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v8_upgrade_backs_up_and_preserves_membership_geometry_and_preferences() {
        let path = temp("v8");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(include_str!("fixtures/v8.sql"))
                .unwrap();
            connection.execute_batch("INSERT INTO panels VALUES (1,'Work','desktop',NULL,15,25,400,300,1,1,'mica',NULL,NULL);
                INSERT INTO panels VALUES (2,'Folder','folder','C:\\Mapped',500,25,400,300,0,0,'mica',NULL,NULL);
                INSERT INTO panel_theme VALUES (1,'dark');
                INSERT INTO panel_layer VALUES (1,1);
                INSERT INTO panel_behavior VALUES (1,1);
                INSERT INTO metadata VALUES ('sentinel','keep');").unwrap();
            let mut item = DesktopItem::new(
                ShellIdentity::Namespace {
                    parsing_name: "test:one".into(),
                },
                "One",
            );
            item.set_placement(DesktopPlacement::Pane {
                pane_id: PanelId::new(1),
                position: GridPosition::new(2, 0),
            });
            let tx = connection.unchecked_transaction().unwrap();
            insert_desktop_items(&tx, &[item]).unwrap();
            tx.commit().unwrap();
        }
        let mut store = WorkspaceStore::open(&path).unwrap();
        let workspace = store.load_workspace().unwrap();
        let panel = workspace.panel(PanelId::new(1)).unwrap();
        assert_eq!(panel.rect(), RectDip::new(15.0, 25.0, 400.0, 300.0));
        assert!(panel.collapsed() && panel.locked() && panel.always_on_top() && panel.auto_hide());
        assert_eq!(panel.theme(), desktop_core::PanelTheme::Dark);
        assert_eq!(
            workspace.panel(PanelId::new(2)).unwrap().folder(),
            Some(Path::new(r"C:\Mapped"))
        );
        assert_eq!(workspace.desktop_items().len(), 1);
        assert_eq!(
            workspace.desktop_items()[0].placement(),
            &DesktopPlacement::Pane {
                pane_id: PanelId::new(1),
                position: GridPosition::new(2, 0)
            }
        );
        store.save_workspace(&workspace).unwrap();
        assert_eq!(store.load_workspace().unwrap(), workspace);
        assert_eq!(
            store.preference("sentinel").unwrap().as_deref(),
            Some("keep")
        );
        assert_eq!(
            store.preference("schema_version").unwrap().as_deref(),
            Some("10")
        );
        drop(store);
        let prefix = format!(
            "{}.before-v10-",
            path.file_stem().unwrap().to_string_lossy()
        );
        let backup = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .unwrap()
            .path();
        let original = Connection::open(&backup).unwrap();
        assert_eq!(
            original
                .query_row(
                    "SELECT value FROM metadata WHERE key='schema_version'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "8"
        );
        drop(original);
        std::fs::remove_file(backup).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn unsupported_v8_path_collections_are_not_silently_discarded() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(include_str!("fixtures/v8.sql"))
            .unwrap();
        connection.execute_batch("INSERT INTO panels VALUES (1,'Manual','manual','1',0,0,400,300,0,0,'mica',NULL,NULL);
            INSERT INTO panel_items VALUES (1,0,'C:\\keep.txt');").unwrap();
        assert!(initialize_schema(&connection).is_err());
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM metadata WHERE key='schema_version'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "8"
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM panel_items", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    fn temp(label: &str) -> PathBuf {
        unique_backup_path(&std::env::temp_dir().join("lucidpane-recovery"), label)
    }
    #[test]
    fn backup_restores_layout_preferences_and_monitor_profiles() {
        let path = temp("roundtrip");
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let mut workspace = Workspace::new();
        let id = PanelId::new(42);
        let rect = RectDip::new(-800.0, 50.0, 480.0, 320.0);
        workspace.add_panel(Panel::new(id, "项目", rect)).unwrap();
        store.save_workspace(&workspace).unwrap();
        store.save_preference("search_enabled", "0").unwrap();
        store
            .save_monitor_layout("two displays", &[(id, rect)])
            .unwrap();
        store.export_backup(&path).unwrap();
        assert!(store.export_backup(&path).is_err());
        store.save_workspace(&Workspace::new()).unwrap();
        store.save_preference("search_enabled", "1").unwrap();
        store.restore_backup(&path).unwrap();
        assert_eq!(store.load_workspace().unwrap(), workspace);
        assert_eq!(
            store.preference("search_enabled").unwrap().as_deref(),
            Some("0")
        );
        assert_eq!(
            store.monitor_layout("two displays").unwrap(),
            vec![(id, rect)]
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn invalid_restore_does_not_change_live_configuration_or_source() {
        let path = temp("invalid");
        std::fs::write(&path, b"not a SQLite database").unwrap();
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        store.save_preference("sentinel", "keep").unwrap();
        assert!(store.restore_backup(&path).is_err());
        assert_eq!(
            store.preference("sentinel").unwrap().as_deref(),
            Some("keep")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"not a SQLite database");
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn invalid_display_profile_is_rejected_before_restore() {
        let path = temp("bad-profile");
        let mut live = WorkspaceStore::open_in_memory().unwrap();
        live.save_preference("sentinel", "keep").unwrap();
        {
            let source = WorkspaceStore::open(&path).unwrap();
            source
                .connection
                .execute(
                    "INSERT INTO monitor_layouts VALUES ('bad',1,0,0,-100,100)",
                    [],
                )
                .unwrap();
        }
        assert!(live.restore_backup(&path).is_err());
        assert_eq!(
            live.preference("sentinel").unwrap().as_deref(),
            Some("keep")
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn v9_migration_preserves_data_and_creates_original_backup() {
        let path = temp("v9");
        {
            let store = WorkspaceStore::open(&path).unwrap();
            store.save_preference("sentinel", "v9 data").unwrap();
            store.connection.execute_batch("DROP TABLE monitor_layouts; UPDATE metadata SET value='9' WHERE key='schema_version'").unwrap();
        }
        {
            let store = WorkspaceStore::open(&path).unwrap();
            assert_eq!(
                store.preference("schema_version").unwrap().as_deref(),
                Some("10")
            );
            assert_eq!(
                store.preference("sentinel").unwrap().as_deref(),
                Some("v9 data")
            );
            assert!(store.monitor_layout("unknown").unwrap().is_empty());
        }
        let prefix = format!(
            "{}.before-v10-",
            path.file_stem().unwrap().to_string_lossy()
        );
        let backups: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .map(|e| e.path())
            .collect();
        assert_eq!(backups.len(), 1);
        let original = Connection::open(&backups[0]).unwrap();
        assert_eq!(
            original
                .query_row(
                    "SELECT value FROM metadata WHERE key='schema_version'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "9"
        );
        drop(original);
        std::fs::remove_file(&backups[0]).unwrap();
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn display_profiles_do_not_overwrite_each_other() {
        let mut store = WorkspaceStore::open_in_memory().unwrap();
        let id = PanelId::new(1);
        let a = RectDip::new(10.0, 20.0, 200.0, 300.0);
        let b = RectDip::new(-900.0, 0.0, 200.0, 300.0);
        store.save_monitor_layout("single", &[(id, a)]).unwrap();
        store.save_monitor_layout("dual", &[(id, b)]).unwrap();
        assert_eq!(store.monitor_layout("single").unwrap(), vec![(id, a)]);
        assert_eq!(store.monitor_layout("dual").unwrap(), vec![(id, b)]);
    }
}

impl WorkspaceStore {
    /// Number of row changes made through this connection, used to avoid redundant snapshots.
    #[must_use]
    pub fn change_count(&self) -> u64 {
        self.connection.total_changes()
    }
    /// Creates a consistent standalone database snapshot, including preferences.
    /// # Errors
    /// Rejects existing destinations and reports SQLite or path errors.
    pub fn export_backup(&self, path: &Path) -> Result<(), StoreError> {
        if path.exists() {
            return Err(StoreError::InvalidData(
                "backup destination already exists".into(),
            ));
        }
        let path = path
            .to_str()
            .ok_or_else(|| StoreError::InvalidData("invalid backup path".into()))?;
        self.connection.execute("VACUUM main INTO ?1", [path])?;
        Ok(())
    }

    /// Validates a backup in isolation before replacing the live database atomically.
    /// # Errors
    /// Invalid databases, unsupported schemas and failed restores leave the live data intact.
    pub fn restore_backup(&mut self, path: &Path) -> Result<(), StoreError> {
        let source = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let integrity: String = source.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(StoreError::InvalidData(integrity));
        }
        let mut copy = Connection::open_in_memory()?;
        {
            let backup = rusqlite::backup::Backup::new(&source, &mut copy)?;
            if backup.step(-1)? != rusqlite::backup::StepResult::Done {
                return Err(StoreError::InvalidData("backup is busy".into()));
            }
        }
        let validated = Self::from_connection(copy)?;
        validated.load_workspace()?;
        for table in ["panel_theme", "panel_layer", "panel_behavior"] {
            let cascades: i64 = validated.connection.query_row(
                "SELECT count(*) FROM pragma_foreign_key_list(?1) WHERE \"table\"='panels' AND on_delete='CASCADE'",
                [table], |r| r.get(0))?;
            if cascades != 1 {
                return Err(StoreError::InvalidData(
                    "backup has an incompatible table schema".into(),
                ));
            }
        }
        let topologies: Vec<String> = validated
            .connection
            .prepare("SELECT DISTINCT topology FROM monitor_layouts")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        for topology in topologies {
            validated.monitor_layout(&topology)?;
        }
        let violations: i64 = validated.connection.query_row(
            "SELECT count(*) FROM pragma_foreign_key_check",
            [],
            |r| r.get(0),
        )?;
        if violations != 0 {
            return Err(StoreError::InvalidData("invalid backup references".into()));
        }
        let backup = rusqlite::backup::Backup::new(&validated.connection, &mut self.connection)?;
        if backup.step(-1)? != rusqlite::backup::StepResult::Done {
            return Err(StoreError::InvalidData("restore is busy".into()));
        }
        Ok(())
    }

    /// Saves geometry for one display arrangement without changing other arrangements.
    /// # Errors
    /// Reports database failures.
    pub fn save_monitor_layout(
        &mut self,
        topology: &str,
        layout: &[(PanelId, RectDip)],
    ) -> Result<(), StoreError> {
        let tx = self.connection.transaction()?;
        tx.execute("DELETE FROM monitor_layouts WHERE topology=?1", [topology])?;
        for (id, r) in layout {
            let id = i64::try_from(id.get())
                .map_err(|_| StoreError::InvalidData("invalid panel id".into()))?;
            tx.execute(
                "INSERT INTO monitor_layouts VALUES (?1,?2,?3,?4,?5,?6)",
                params![topology, id, r.x, r.y, r.width, r.height],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Loads saved geometry for a display arrangement.
    /// # Errors
    /// Reports invalid geometry or database failures.
    pub fn monitor_layout(&self, topology: &str) -> Result<Vec<(PanelId, RectDip)>, StoreError> {
        let mut stmt = self
            .connection
            .prepare("SELECT panel_id,x,y,width,height FROM monitor_layouts WHERE topology=?1")?;
        let rows = stmt.query_map([topology], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                RectDip {
                    x: r.get(1)?,
                    y: r.get(2)?,
                    width: r.get(3)?,
                    height: r.get(4)?,
                },
            ))
        })?;
        rows.map(|row| {
            let (id, r) = row?;
            if id == 0
                || ![r.x, r.y, r.width, r.height]
                    .into_iter()
                    .all(f32::is_finite)
                || r.width <= 0.0
                || r.height <= 0.0
            {
                return Err(StoreError::InvalidData("invalid saved layout".into()));
            }
            Ok((PanelId::new(id), r))
        })
        .collect()
    }
}
