CREATE TABLE metadata (
             key TEXT PRIMARY KEY NOT NULL,
             value TEXT NOT NULL
         );
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
CREATE TABLE panel_behavior (
             panel_id INTEGER PRIMARY KEY REFERENCES panels(id) ON DELETE CASCADE,
             auto_hide INTEGER NOT NULL DEFAULT 0
         );
CREATE TABLE panel_items (
             panel_id INTEGER NOT NULL REFERENCES panels(id) ON DELETE CASCADE,
             item_order INTEGER NOT NULL,
             path TEXT NOT NULL,
             PRIMARY KEY(panel_id, item_order)
         );
CREATE TABLE desktop_items (
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
         );
CREATE TABLE panel_layer (
             panel_id INTEGER PRIMARY KEY REFERENCES panels(id) ON DELETE CASCADE,
             always_on_top INTEGER NOT NULL DEFAULT 0
         );
CREATE TABLE panel_theme (
             panel_id INTEGER PRIMARY KEY REFERENCES panels(id) ON DELETE CASCADE,
             theme TEXT NOT NULL DEFAULT 'system'
         );
INSERT INTO metadata VALUES ('schema_version','8');
