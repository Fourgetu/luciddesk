use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Backdrop {
    Translucent { opacity: f32 },
    Mica,
    MicaAlt,
    Acrylic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PanelId(u64);

impl PanelId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ShellIdentity {
    FileSystem {
        path: PathBuf,
        volume_id: Option<u64>,
        file_id: Option<u128>,
    },
    Namespace {
        parsing_name: String,
    },
}

impl ShellIdentity {
    #[must_use]
    pub fn persistent_key(&self) -> String {
        match self {
            Self::FileSystem {
                path,
                volume_id,
                file_id,
            } => match (volume_id, file_id) {
                (Some(volume_id), Some(file_id)) => {
                    format!("fsid:{volume_id:016x}:{file_id:032x}")
                }
                _ => format!("fs:{}", path.as_os_str().to_string_lossy().to_lowercase()),
            },
            Self::Namespace { parsing_name } => {
                format!("shell:{}", parsing_name.to_lowercase())
            }
        }
    }

    /// Compares two Shell identities using file IDs when available and paths as a migration
    /// fallback for records created before stable file identity was captured.
    #[must_use]
    pub fn equivalent_to(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::FileSystem {
                    path: left_path,
                    volume_id: left_volume,
                    file_id: left_file,
                },
                Self::FileSystem {
                    path: right_path,
                    volume_id: right_volume,
                    file_id: right_file,
                },
            ) => {
                let stable_match = match (left_volume, left_file, right_volume, right_file) {
                    (Some(left_volume), Some(left_file), Some(right_volume), Some(right_file)) => {
                        left_volume == right_volume && left_file == right_file
                    }
                    _ => false,
                };
                stable_match || path_key(left_path) == path_key(right_path)
            }
            (
                Self::Namespace { parsing_name: left },
                Self::Namespace {
                    parsing_name: right,
                },
            ) => left.eq_ignore_ascii_case(right),
            _ => false,
        }
    }

    #[must_use]
    pub fn file_system_path(&self) -> Option<&Path> {
        match self {
            Self::FileSystem { path, .. } => Some(path),
            Self::Namespace { .. } => None,
        }
    }

    #[must_use]
    pub fn activation_name(&self) -> &OsStr {
        match self {
            Self::FileSystem { path, .. } => path.as_os_str(),
            Self::Namespace { parsing_name } => OsStr::new(parsing_name),
        }
    }
}

fn path_key(path: &Path) -> String {
    path.as_os_str().to_string_lossy().to_lowercase()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MonitorId(String);

impl MonitorId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for MonitorId {
    fn default() -> Self {
        Self::new("primary")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointDip {
    pub x: f32,
    pub y: f32,
}

impl PointDip {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridPosition {
    pub column: u32,
    pub row: u32,
}

impl GridPosition {
    #[must_use]
    pub const fn new(column: u32, row: u32) -> Self {
        Self { column, row }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesktopPlacement {
    FreeDesktop {
        monitor: MonitorId,
        position: PointDip,
    },
    Pane {
        pane_id: PanelId,
        position: GridPosition,
    },
}

impl Default for DesktopPlacement {
    fn default() -> Self {
        Self::FreeDesktop {
            monitor: MonitorId::default(),
            position: PointDip::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopItem {
    identity: ShellIdentity,
    display_name: String,
    placement: DesktopPlacement,
}

impl DesktopItem {
    #[must_use]
    pub fn new(identity: ShellIdentity, display_name: impl Into<String>) -> Self {
        Self {
            identity,
            display_name: display_name.into(),
            placement: DesktopPlacement::default(),
        }
    }

    #[must_use]
    pub const fn identity(&self) -> &ShellIdentity {
        &self.identity
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub const fn placement(&self) -> &DesktopPlacement {
        &self.placement
    }

    pub fn set_display_name(&mut self, display_name: impl Into<String>) {
        self.display_name = display_name.into();
    }

    pub fn set_placement(&mut self, placement: DesktopPlacement) {
        self.placement = placement;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectDip {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl RectDip {
    pub const MIN_WIDTH: f32 = 260.0;
    pub const MIN_HEIGHT: f32 = 160.0;

    #[must_use]
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width: width.max(Self::MIN_WIDTH),
            height: height.max(Self::MIN_HEIGHT),
        }
    }
}

impl Default for RectDip {
    fn default() -> Self {
        Self::new(120.0, 120.0, 420.0, 360.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelSource {
    Folder { path: PathBuf },
    DesktopCollection,
    ManualCollection { collection_id: u64 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PanelIcon {
    #[default]
    Automatic,
    Custom(PathBuf),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelTheme { #[default] System, Light, Dark }

#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    theme: PanelTheme,
    always_on_top: bool,
    auto_hide: bool,
    id: PanelId,
    title: String,
    source: PanelSource,
    icon: PanelIcon,
    rect: RectDip,
    collapsed: bool,
    locked: bool,
    backdrop: Backdrop,
    items: Vec<PathBuf>,
}

impl Panel {
    #[must_use]
    pub fn new(id: PanelId, title: impl Into<String>, source: PanelSource, rect: RectDip) -> Self {
        Self {
            id,
            title: title.into(),
            source,
            icon: PanelIcon::Automatic,
            rect,
            collapsed: false,
            auto_hide: false,
            theme: PanelTheme::System,
            always_on_top: false,
            locked: false,
            backdrop: Backdrop::DEFAULT,
            items: Vec::new(),
        }
    }

    #[must_use]
    pub const fn id(&self) -> PanelId {
        self.id
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub const fn source(&self) -> &PanelSource {
        &self.source
    }

    #[must_use]
    pub const fn icon(&self) -> &PanelIcon {
        &self.icon
    }

    #[must_use]
    pub const fn rect(&self) -> RectDip {
        self.rect
    }

    #[must_use]
    pub const fn collapsed(&self) -> bool {
        self.collapsed
    }

    #[must_use]
    pub const fn auto_hide(&self) -> bool {
        self.auto_hide
    }

    #[must_use]
    pub const fn always_on_top(&self) -> bool { self.always_on_top }

    #[must_use]
    pub const fn theme(&self) -> PanelTheme { self.theme }
    pub const fn set_theme(&mut self, theme: PanelTheme) { self.theme = theme; }

    pub const fn set_always_on_top(&mut self, enabled: bool) { self.always_on_top = enabled; }

    pub const fn set_auto_hide(&mut self, enabled: bool) {
        self.auto_hide = enabled;
    }

    #[must_use]
    pub const fn locked(&self) -> bool {
        self.locked
    }

    #[must_use]
    pub const fn backdrop(&self) -> Backdrop {
        self.backdrop
    }

    #[must_use]
    pub fn item_paths(&self) -> &[PathBuf] {
        &self.items
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    pub fn set_source(&mut self, source: PanelSource) {
        self.source = source;
    }

    pub fn set_icon(&mut self, icon: PanelIcon) {
        self.icon = icon;
    }

    pub fn set_rect(&mut self, rect: RectDip) {
        self.rect = RectDip::new(rect.x, rect.y, rect.width, rect.height);
    }

    pub const fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }

    pub const fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
    }

    pub const fn set_backdrop(&mut self, backdrop: Backdrop) {
        self.backdrop = backdrop;
    }

    /// Adds a logical item reference without moving or copying the real file.
    ///
    /// Returns `true` when the path was new to this pane.
    pub fn add_item(&mut self, path: PathBuf) -> bool {
        if self
            .items
            .iter()
            .any(|existing| paths_equal(existing, &path))
        {
            return false;
        }
        self.items.push(path);
        true
    }

    /// Replaces the logical item order, discarding case-insensitive duplicates.
    pub fn replace_items(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        self.items.clear();
        for path in paths {
            self.add_item(path);
        }
    }

    pub fn remove_item(&mut self, path: &std::path::Path) -> bool {
        let Some(index) = self
            .items
            .iter()
            .position(|existing| paths_equal(existing, path))
        else {
            return false;
        };
        self.items.remove(index);
        true
    }

    /// Moves one item to another grid position while preserving every reference.
    pub fn move_item(&mut self, from: usize, to: usize) -> bool {
        if from >= self.items.len() || to >= self.items.len() || from == to {
            return false;
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        true
    }
}

fn paths_equal(left: &std::path::Path, right: &std::path::Path) -> bool {
    left.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workspace {
    panels: Vec<Panel>,
    desktop_items: Vec<DesktopItem>,
}

impl Workspace {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            panels: Vec::new(),
            desktop_items: Vec::new(),
        }
    }

    /// Builds a workspace from an existing panel collection.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceError::DuplicatePanel`] when panel IDs are not unique.
    pub fn from_panels(panels: Vec<Panel>) -> Result<Self, WorkspaceError> {
        let mut ids = HashSet::with_capacity(panels.len());
        for panel in &panels {
            if !ids.insert(panel.id()) {
                return Err(WorkspaceError::DuplicatePanel(panel.id()));
            }
        }
        Ok(Self {
            panels,
            desktop_items: Vec::new(),
        })
    }

    #[must_use]
    pub fn panels(&self) -> &[Panel] {
        &self.panels
    }

    #[must_use]
    pub fn desktop_items(&self) -> &[DesktopItem] {
        &self.desktop_items
    }

    pub fn desktop_items_mut(&mut self) -> &mut [DesktopItem] {
        &mut self.desktop_items
    }

    #[must_use]
    pub fn desktop_item(&self, identity: &ShellIdentity) -> Option<&DesktopItem> {
        self.desktop_items
            .iter()
            .find(|item| item.identity().equivalent_to(identity))
    }

    pub fn desktop_item_mut(&mut self, identity: &ShellIdentity) -> Option<&mut DesktopItem> {
        self.desktop_items
            .iter_mut()
            .find(|item| item.identity().equivalent_to(identity))
    }

    /// Reconciles a fresh Shell inventory while preserving placement for surviving identities.
    pub fn reconcile_desktop_items(&mut self, inventory: impl IntoIterator<Item = DesktopItem>) {
        let previous = std::mem::take(&mut self.desktop_items);
        self.desktop_items = inventory
            .into_iter()
            .map(|mut incoming| {
                if let Some(existing) = previous
                    .iter()
                    .find(|existing| existing.identity().equivalent_to(incoming.identity()))
                {
                    incoming.set_placement(existing.placement().clone());
                }
                incoming
            })
            .collect();
    }

    /// Adds a panel to this workspace.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceError::DuplicatePanel`] when the ID is already present.
    pub fn add_panel(&mut self, panel: Panel) -> Result<(), WorkspaceError> {
        if self.panel(panel.id()).is_some() {
            return Err(WorkspaceError::DuplicatePanel(panel.id()));
        }
        self.panels.push(panel);
        Ok(())
    }

    #[must_use]
    pub fn panel(&self, id: PanelId) -> Option<&Panel> {
        self.panels.iter().find(|panel| panel.id() == id)
    }

    pub fn panel_mut(&mut self, id: PanelId) -> Option<&mut Panel> {
        self.panels.iter_mut().find(|panel| panel.id() == id)
    }

    pub fn remove_panel(&mut self, id: PanelId) -> Option<Panel> {
        let index = self.panels.iter().position(|panel| panel.id() == id)?;
        Some(self.panels.remove(index))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceError {
    DuplicatePanel(PanelId),
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePanel(id) => write!(formatter, "panel {} already exists", id.get()),
        }
    }
}

impl std::error::Error for WorkspaceError {}

#[cfg(test)]
mod tests {
    use super::{
        DesktopItem, DesktopPlacement, GridPosition, Panel, PanelIcon, PanelId, PanelSource,
        RectDip, ShellIdentity, Workspace, WorkspaceError,
    };
    use std::path::PathBuf;

    fn panel(id: u64) -> Panel {
        Panel::new(
            PanelId::new(id),
            format!("Panel {id}"),
            PanelSource::Folder {
                path: PathBuf::from(r"C:\Users\Test\Desktop"),
            },
            RectDip::default(),
        )
    }

    #[test]
    fn rect_enforces_minimum_size() {
        let rect = RectDip::new(1.0, 2.0, 20.0, 30.0);
        assert!((rect.width - RectDip::MIN_WIDTH).abs() < f32::EPSILON);
        assert!((rect.height - RectDip::MIN_HEIGHT).abs() < f32::EPSILON);
    }

    #[test]
    fn workspace_rejects_duplicate_ids() {
        let result = Workspace::from_panels(vec![panel(7), panel(7)]);
        assert_eq!(result, Err(WorkspaceError::DuplicatePanel(PanelId::new(7))));
    }

    #[test]
    fn workspace_can_add_find_and_remove_panel() {
        let mut workspace = Workspace::new();
        workspace
            .add_panel(panel(1))
            .expect("panel should be added");
        assert_eq!(workspace.panel(PanelId::new(1)).unwrap().title(), "Panel 1");
        assert!(workspace.remove_panel(PanelId::new(1)).is_some());
        assert!(workspace.panels().is_empty());
    }

    #[test]
    fn panel_supports_a_custom_header_icon() {
        let mut panel = panel(1);
        let icon = PanelIcon::Custom(PathBuf::from(r"C:\Icons\work.ico"));
        panel.set_icon(icon.clone());
        assert_eq!(panel.icon(), &icon);
    }

    #[test]
    fn manual_panel_starts_empty_and_deduplicates_item_references() {
        let mut panel = Panel::new(
            PanelId::new(1),
            "New Pane",
            PanelSource::ManualCollection { collection_id: 1 },
            RectDip::default(),
        );
        assert!(panel.item_paths().is_empty());

        assert!(panel.add_item(PathBuf::from(r"C:\Users\Test\Desktop\Editor.lnk")));
        assert!(!panel.add_item(PathBuf::from(r"c:\users\test\desktop\EDITOR.LNK")));
        assert_eq!(panel.item_paths().len(), 1);
    }

    #[test]
    fn manual_panel_items_can_be_reordered() {
        let mut panel = Panel::new(
            PanelId::new(1),
            "New Pane",
            PanelSource::ManualCollection { collection_id: 1 },
            RectDip::default(),
        );
        panel.add_item(PathBuf::from("one.lnk"));
        panel.add_item(PathBuf::from("two.lnk"));
        panel.add_item(PathBuf::from("three.lnk"));

        assert!(panel.move_item(0, 2));
        assert_eq!(
            panel.item_paths(),
            &[
                PathBuf::from("two.lnk"),
                PathBuf::from("three.lnk"),
                PathBuf::from("one.lnk")
            ]
        );
        assert!(!panel.move_item(4, 0));
    }

    #[test]
    fn replacing_items_preserves_order_and_removes_duplicates() {
        let mut panel = Panel::new(
            PanelId::new(1),
            "Desktop",
            PanelSource::DesktopCollection,
            RectDip::default(),
        );
        panel.add_item(PathBuf::from("old.lnk"));

        panel.replace_items([
            PathBuf::from("two.lnk"),
            PathBuf::from("ONE.lnk"),
            PathBuf::from("one.LNK"),
        ]);

        assert_eq!(
            panel.item_paths(),
            &[PathBuf::from("two.lnk"), PathBuf::from("ONE.lnk")]
        );
    }

    #[test]
    fn desktop_reconciliation_preserves_membership_without_moving_files() {
        let identity = ShellIdentity::FileSystem {
            path: PathBuf::from(r"C:\Users\Test\Desktop\Editor.lnk"),
            volume_id: None,
            file_id: None,
        };
        let mut existing = DesktopItem::new(identity.clone(), "Old Editor Name");
        existing.set_placement(DesktopPlacement::Pane {
            pane_id: PanelId::new(7),
            position: GridPosition::new(2, 3),
        });
        let mut workspace = Workspace::new();
        workspace.reconcile_desktop_items([existing]);

        workspace.reconcile_desktop_items([
            DesktopItem::new(identity.clone(), "Editor"),
            DesktopItem::new(
                ShellIdentity::Namespace {
                    parsing_name: "::{645FF040-5081-101B-9F08-00AA002F954E}".into(),
                },
                "Recycle Bin",
            ),
        ]);

        let editor = workspace.desktop_item(&identity).unwrap();
        assert_eq!(editor.display_name(), "Editor");
        assert_eq!(
            editor.placement(),
            &DesktopPlacement::Pane {
                pane_id: PanelId::new(7),
                position: GridPosition::new(2, 3),
            }
        );
        assert_eq!(
            identity.file_system_path(),
            Some(PathBuf::from(r"C:\Users\Test\Desktop\Editor.lnk").as_path())
        );
        assert_eq!(workspace.desktop_items().len(), 2);
    }

    #[test]
    fn stable_file_identity_preserves_placement_across_a_rename() {
        let before = ShellIdentity::FileSystem {
            path: PathBuf::from(r"C:\Users\Test\Desktop\Before.txt"),
            volume_id: Some(11),
            file_id: Some(22),
        };
        let after = ShellIdentity::FileSystem {
            path: PathBuf::from(r"C:\Users\Test\Desktop\After.txt"),
            volume_id: Some(11),
            file_id: Some(22),
        };
        let placement = DesktopPlacement::Pane {
            pane_id: PanelId::new(3),
            position: GridPosition::new(1, 4),
        };
        let mut existing = DesktopItem::new(before, "Before");
        existing.set_placement(placement.clone());
        let mut workspace = Workspace::new();
        workspace.reconcile_desktop_items([existing]);

        workspace.reconcile_desktop_items([DesktopItem::new(after.clone(), "After")]);

        let renamed = workspace.desktop_item(&after).unwrap();
        assert_eq!(renamed.display_name(), "After");
        assert_eq!(renamed.placement(), &placement);
        assert_eq!(renamed.identity(), &after);
    }
}

impl Backdrop {
    pub const DEFAULT: Self = Self::Mica;

    #[must_use]
    pub const fn translucent() -> Self {
        Self::Translucent { opacity: 0.86 }
    }

    #[must_use]
    pub const fn kind(self) -> BackdropKind {
        match self {
            Self::Translucent { .. } => BackdropKind::Translucent,
            Self::Mica => BackdropKind::Mica,
            Self::MicaAlt => BackdropKind::MicaAlt,
            Self::Acrylic => BackdropKind::Acrylic,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        self.kind().label()
    }
}

impl Default for Backdrop {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackdropKind {
    Mica,
    MicaAlt,
    Acrylic,
    Translucent,
}

impl BackdropKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mica => "Mica",
            Self::MicaAlt => "Mica Alt",
            Self::Acrylic => "Desktop Acrylic",
            Self::Translucent => "Translucent",
        }
    }
}
