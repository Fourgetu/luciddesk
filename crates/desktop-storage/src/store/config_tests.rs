use desktop_core::{DesktopItem, DesktopPlacement, GridPosition, ShellIdentity};
use super::*;

fn open() -> (tempfile::TempDir, WorkspaceStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(&dir.path().join("workspace.db")).unwrap();
    (dir, store)
}

#[test]
fn icon_grid_settings_round_trip_and_reject_invalid_dimensions() {
    let (dir, store) = open();
    let options = desktop_core::PaneOptions {
        grid_scale: 150.0,
        ..desktop_core::PaneOptions::DEFAULT
    };
    store.save_pane_options(options).unwrap();
    drop(store);
    let reopened = WorkspaceStore::open(&dir.path().join("workspace.db")).unwrap();
    assert_eq!(reopened.load_workspace().unwrap().pane_options(), options);
    let source = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(source.contains("grid_scale = 150.0"));
    for value in ["0.0", "201.0", "nan", "inf"] {
        let invalid = source.replace("grid_scale = 150.0", &format!("grid_scale = {value}"));
        assert!(config::ConfigFile::parse(dir.path().join("config.toml"), invalid).is_err());
    }
}

#[test]
fn new_files_ignore_legacy_database_and_keep_preferences_out_of_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("hook-desktop.db");
    std::fs::write(&old, b"old development database").unwrap();
    let store = WorkspaceStore::open(&dir.path().join("workspace.db")).unwrap();
    assert_eq!(std::fs::read(old).unwrap(), b"old development database");
    store.save_preference("search_enabled", "0").unwrap();
    let doc = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(doc.contains("enabled = false"));
    assert_eq!(store.connection.query_row("SELECT count(*) FROM metadata WHERE key IN ('search_enabled','pane_options','appearance')",[],|r|r.get::<_,u32>(0)).unwrap(),0);
}

#[test]
fn external_edits_require_reload_preserve_comments_and_reject_invalid_values() {
    let (_dir, store) = open();
    store.save_preference("search_enabled", "1").unwrap();
    let path = store.config_path().unwrap();
    let original = std::fs::read_to_string(&path).unwrap();
    let edited = original.replace(
        "corner_radius = 6.0",
        "corner_radius = 3.25 # personal choice",
    );
    std::fs::write(&path, &edited).unwrap();
    assert!(
        store
            .save_preference("search_enabled", "0")
            .unwrap_err()
            .to_string()
            .contains("重新加载")
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    store.reload_config().unwrap();
    assert_eq!(
        store.load_workspace().unwrap().pane_options().corner_radius,
        3.25
    );
    store.save_preference("search_enabled", "0").unwrap();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("# personal choice")
    );
    let invalid = edited.replace("3.25", "100.0");
    std::fs::write(&path, &invalid).unwrap();
    assert!(
        store
            .reload_config()
            .unwrap_err()
            .to_string()
            .contains("panel_defaults.corner_radius")
    );
    assert_eq!(
        store.load_workspace().unwrap().pane_options().corner_radius,
        3.25
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
}

#[test]
fn unchanged_workspace_does_not_rewrite_database_or_config() {
    let (_dir, mut store) = open();
    let mut workspace = store.load_workspace().unwrap();
    let id = PanelId::new(1);
    workspace
        .add_panel(Panel::new(
            id,
            "Example",
            RectDip::new(10.0, 20.0, 300.0, 200.0),
        ))
        .unwrap();
    store.save_workspace(&workspace).unwrap();
    let changes = store.change_count();
    let path = store.config_path().unwrap();
    let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
    store.save_workspace(&workspace).unwrap();
    assert_eq!(store.change_count(), changes);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), stamp);
}

#[test]
fn backup_contains_config_and_restore_replaces_both() {
    let (dir, mut store) = open();
    let mut workspace = store.load_workspace().unwrap();
    workspace
        .add_panel(Panel::new(
            PanelId::new(1),
            "Keep",
            RectDip::new(10.0, 20.0, 300.0, 200.0),
        ))
        .unwrap();
    store.save_workspace(&workspace).unwrap();
    store.save_preference("search_enabled", "0").unwrap();
    let backup = dir.path().join("snapshot.db");
    store.export_backup(&backup).unwrap();
    let bytes = std::fs::read(&backup).unwrap();
    store.save_preference("search_enabled", "1").unwrap();
    store.save_workspace(&Workspace::new()).unwrap();
    store.restore_backup(&backup).unwrap();
    assert_eq!(
        store.preference("search_enabled").unwrap().as_deref(),
        Some("0")
    );
    assert_eq!(store.load_workspace().unwrap().panels().len(), 1);
    assert_eq!(std::fs::read(&backup).unwrap(), bytes);
    drop(store);
    let store = WorkspaceStore::open(&dir.path().join("workspace.db")).unwrap();
    assert_eq!(
        store.preference("search_enabled").unwrap().as_deref(),
        Some("0")
    );
}

#[test]
fn background_snapshot_is_independent_and_export_replaces_only_when_requested() {
    let (dir, store) = open();
    store.save_preference("search_enabled", "0").unwrap();
    let snapshot = store.backup_snapshot().unwrap();
    let content = snapshot.backup_content().unwrap();
    store.save_preference("search_enabled", "1").unwrap();
    assert_eq!(snapshot.backup_content().unwrap(), content);
    let path = dir.path().join("export.db");
    snapshot.export_backup(&path).unwrap();
    assert_eq!(WorkspaceStore::backup_file_content(&path).unwrap(), content);
    assert_eq!(WorkspaceStore::inspect_backup(&path).unwrap(), 0);
    assert!(store.export_backup(&path).is_err());
    store.export_backup_replace(&path).unwrap();
    assert_ne!(WorkspaceStore::backup_file_content(&path).unwrap(), content);
    let original = std::fs::read(&path).unwrap();
    std::fs::write(store.config_path().unwrap(), "invalid configuration").unwrap();
    assert!(store.export_backup_replace(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn corrupted_backup_config_is_rejected_before_changing_live_state() {
    let (dir, mut store) = open();
    let backup = dir.path().join("snapshot.db");
    store.export_backup(&backup).unwrap();
    let conn = Connection::open(&backup).unwrap();
    conn.execute(
        "UPDATE metadata SET value='config_version = 999' WHERE key='backup_config'",
        [],
    )
    .unwrap();
    drop(conn);
    let before = std::fs::read(store.config_path().unwrap()).unwrap();
    let changes = store.change_count();
    assert!(store.restore_backup(&backup).is_err());
    assert_eq!(store.change_count(), changes);
    assert_eq!(std::fs::read(store.config_path().unwrap()).unwrap(), before);
}

#[test]
fn missing_fields_default_and_malformed_tables_do_not() {
    let doc = "config_version = 1\n[panel_defaults]\ncorner_radius=2.5"
        .parse()
        .unwrap();
    let values = config::decode(&doc).unwrap();
    assert_eq!(values["pane_options"], "2.5|true|true|auto|false|100");
    for invalid in [
        "config_version=1\nappearance=2",
        "config_version=1\n[appearance.solid]\ncolor='blue'",
        "config_version=1\n[appearance.mica]\nstrength=3.5",
    ] {
        assert!(config::decode(&invalid.parse().unwrap()).is_err());
    }
}

#[test]
fn reloaded_appearance_updates_inherited_panels_and_preserves_overrides() {
    let (_dir, mut store) = open();
    let mut workspace = store.load_workspace().unwrap();
    let (theme, backdrop) = workspace.appearance().unwrap();
    let mut inherited = Panel::new(
        PanelId::new(1),
        "Default",
        RectDip::new(0.0, 0.0, 300.0, 200.0),
    );
    inherited.set_theme(theme);
    inherited.set_backdrop(backdrop);
    let mut custom = Panel::new(
        PanelId::new(2),
        "Custom",
        RectDip::new(400.0, 0.0, 300.0, 200.0),
    );
    custom.set_theme(desktop_core::PanelTheme::Light);
    custom.set_backdrop(Backdrop::Solid {
        color: 0xff0000,
        opacity: 0.8,
    });
    workspace.add_panel(inherited).unwrap();
    workspace.add_panel(custom.clone()).unwrap();
    *workspace.panel_mut(PanelId::new(2)).unwrap() = custom.clone();
    store.save_workspace(&workspace).unwrap();
    let path = store.config_path().unwrap();
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("theme = \"system\"", "theme = \"dark\"")
        .replace("strength = 50", "strength = 25");
    std::fs::write(path, text).unwrap();
    store.reload_config().unwrap();
    let loaded = store.load_workspace().unwrap();
    assert_eq!(
        loaded.panel(PanelId::new(1)).unwrap().theme(),
        desktop_core::PanelTheme::Dark
    );
    assert_eq!(
        loaded.panel(PanelId::new(1)).unwrap().backdrop(),
        Backdrop::Mica.with_strength(25)
    );
    assert_eq!(loaded.panel(PanelId::new(2)).unwrap(), &custom);
}

#[test]
fn interrupted_restore_finishes_config_write_on_reopen() {
    let (dir, store) = open();
    let pending = std::fs::read_to_string(store.config_path().unwrap())
        .unwrap()
        .replace("corner_radius = 6.0", "corner_radius = 9.0");
    store
        .connection
        .execute(
            "INSERT INTO metadata VALUES ('pending_config',?1)",
            [&pending],
        )
        .unwrap();
    drop(store);
    let store = WorkspaceStore::open(&dir.path().join("workspace.db")).unwrap();
    assert_eq!(
        store.load_workspace().unwrap().pane_options().corner_radius,
        9.0
    );
    assert!(store.preference("pending_config").unwrap().is_none());
}

#[test]
fn failed_workspace_commit_preserves_configuration() {
    let (_dir, mut store) = open();
    let path = store.config_path().unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut workspace = store.load_workspace().unwrap();
    let mut options = workspace.pane_options();
    options.corner_radius = 10.0;
    workspace.set_pane_options(options);
    let mut item = DesktopItem::new(
        ShellIdentity::Namespace {
            parsing_name: "test:orphan".into(),
        },
        "Orphan",
    );
    item.set_placement(DesktopPlacement::Pane {
        pane_id: PanelId::new(999),
        position: GridPosition::new(0, 0),
    });
    workspace.reconcile_desktop_items([item]);
    assert!(store.save_workspace(&workspace).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        store.load_workspace().unwrap().pane_options().corner_radius,
        6.0
    );
    assert!(store.load_workspace().unwrap().desktop_items().is_empty());
}


#[test]
fn language_round_trip_and_legacy_default() {
    let (dir, store) = open();
    assert_eq!(store.preference("language").unwrap().as_deref(), Some("system"));
    for language in ["zh-CN", "zh-TW", "en-US", "ja-JP", "ko-KR", "de-DE", "ru-RU", "system"] {
        store.save_preference("language", language).unwrap();
        let reopened = WorkspaceStore::open(&dir.path().join("workspace.db")).unwrap();
        assert_eq!(reopened.preference("language").unwrap().as_deref(), Some(language));
    }
    let config = dir.path().join("config.toml");
    let original = std::fs::read_to_string(&config).unwrap();
    assert!(store.save_preference("language", "invalid").is_err());
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
    drop(store);
    std::fs::write(&config, original.replace("language = \"system\"\n", "")).unwrap();
    let reopened = WorkspaceStore::open(&dir.path().join("workspace.db")).unwrap();
    assert_eq!(reopened.preference("language").unwrap().as_deref(), Some("system"));
}

#[test]
fn show_panels_hotkey_defaults_disabled_and_persists_independently() {
    let (dir, store) = open();
    assert_eq!(store.preference("show_panels_enabled").unwrap().as_deref(), Some("0"));
    assert_eq!(store.preference("show_panels_hotkey").unwrap().as_deref(), Some("68:3"));
    store.save_preference("show_panels_hotkey", "74:5").unwrap();
    store.save_preference("show_panels_enabled", "1").unwrap();
    let reopened = WorkspaceStore::open(&dir.path().join("workspace.db")).unwrap();
    assert_eq!(reopened.preference("show_panels_hotkey").unwrap().as_deref(), Some("74:5"));
    assert_eq!(reopened.preference("show_panels_enabled").unwrap().as_deref(), Some("1"));
    assert_eq!(reopened.preference("search_enabled").unwrap().as_deref(), Some("0"));
    assert_eq!(reopened.preference("search_hotkey").unwrap().as_deref(), Some("32:3"));
    assert!(store.save_preference("show_panels_enabled", "yes").is_err());
    assert!(store.save_preference("show_panels_hotkey", "32:4").is_err());
    drop(reopened);
    drop(store);
    let path = dir.path().join("config.toml");
    let mut doc = std::fs::read_to_string(&path).unwrap().parse::<toml_edit::DocumentMut>().unwrap();
    doc.remove("show_panels");
    std::fs::write(&path, doc.to_string()).unwrap();
    let legacy = WorkspaceStore::open(&dir.path().join("workspace.db")).unwrap();
    assert_eq!(legacy.preference("show_panels_enabled").unwrap().as_deref(), Some("0"));
}
