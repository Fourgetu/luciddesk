//! Workspace membership and appearance defaults.
use crate::{
    Backdrop, DesktopItem, DesktopPlacement, GridPosition, PaneOptions, Panel, PanelId, PanelTheme,
    ShellIdentity,
};
use std::collections::HashSet;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    appearance: Option<(PanelTheme, Backdrop)>,
    pane_options: PaneOptions,
    panels: Vec<Panel>,
    desktop_items: Vec<DesktopItem>,
    tabs: Vec<PaneTabs>,
}

/// Ordered content panes sharing one window. Content IDs keep their identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneTabs {
    pub members: Vec<PanelId>,
    pub active: PanelId,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    #[must_use]
    pub const fn pane_options(&self) -> PaneOptions {
        self.pane_options
    }
    pub fn set_pane_options(&mut self, options: PaneOptions) {
        self.pane_options = options;
    }

    #[must_use]
    pub const fn new() -> Self {
        Self {
            panels: Vec::new(),
            appearance: None,
            pane_options: PaneOptions::DEFAULT,
            desktop_items: Vec::new(),
            tabs: Vec::new(),
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
            appearance: None,
            pane_options: PaneOptions::DEFAULT,
            desktop_items: Vec::new(),
            tabs: Vec::new(),
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

    /// Reconciles a fresh Shell inventory while preserving placement for surviving
    /// identities.
    ///
    /// Returns the identities that were not known before, in inventory order: newly
    /// created, copied or downloaded entries.
    pub fn reconcile_desktop_items(
        &mut self,
        inventory: impl IntoIterator<Item = DesktopItem>,
    ) -> Vec<ShellIdentity> {
        let previous = std::mem::take(&mut self.desktop_items);
        let mut fresh = Vec::new();
        self.desktop_items = inventory
            .into_iter()
            .map(|mut incoming| {
                if let Some(existing) = previous
                    .iter()
                    .find(|existing| existing.identity().equivalent_to(incoming.identity()))
                {
                    incoming.set_placement(existing.placement().clone());
                } else {
                    fresh.push(incoming.identity().clone());
                }
                incoming
            })
            .collect();
        fresh
    }

    /// Appends `identities` after the items already filed in `inbox`.
    ///
    /// Identities already placed in any pane are left alone, so an item can never be
    /// counted twice. Returns how many items were moved.
    pub fn append_to_pane(&mut self, inbox: PanelId, identities: &[ShellIdentity]) -> usize {
        if self
            .panel(inbox)
            .is_none_or(|panel| !panel.supports_tabs())
        {
            return 0;
        }
        let mut at = self
            .desktop_items
            .iter()
            .filter_map(|item| match item.placement() {
                DesktopPlacement::Pane { pane_id, position } if *pane_id == inbox => {
                    Some(u64::from(position.column) + 1)
                }
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let mut moved = 0;
        for identity in identities {
            let Some(item) = self
                .desktop_items
                .iter_mut()
                .find(|item| item.identity().equivalent_to(identity))
            else {
                continue;
            };
            if matches!(item.placement(), DesktopPlacement::Pane { .. }) {
                continue;
            }
            item.set_placement(DesktopPlacement::Pane {
                pane_id: inbox,
                position: GridPosition::new(u32::try_from(at).unwrap_or(u32::MAX), 0),
            });
            at += 1;
            moved += 1;
        }
        moved
    }

    /// Moves every item that has never been filed into a pane into `inbox`.
    ///
    /// Reconcile keeps the previous placement of identities it already knows, so an
    /// item that still sits in its default [`DesktopPlacement::FreeDesktop`] is one
    /// the user never filed away: freshly created, copied or downloaded entries.
    /// Each is appended after the items already in the pane, in inventory order.
    ///
    /// Returns how many items were moved.
    pub fn collect_new_items(&mut self, inbox: PanelId) -> usize {
        if self
            .panel(inbox)
            .is_none_or(|panel| !panel.supports_tabs())
        {
            return 0;
        }
        let mut at = self
            .desktop_items
            .iter()
            .filter(|item| {
                matches!(item.placement(), DesktopPlacement::Pane { pane_id, .. } if *pane_id == inbox)
            })
            .count();
        let mut moved = 0;
        for item in &mut self.desktop_items {
            if !matches!(item.placement(), DesktopPlacement::FreeDesktop { .. }) {
                continue;
            }
            item.set_placement(DesktopPlacement::Pane {
                pane_id: inbox,
                position: GridPosition::new(at as u32, 0),
            });
            at += 1;
            moved += 1;
        }
        moved
    }

    /// Adds a panel to this workspace.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceError::DuplicatePanel`] when the ID is already present.
    pub fn add_panel(&mut self, mut panel: Panel) -> Result<(), WorkspaceError> {
        if self.panel(panel.id()).is_some() {
            return Err(WorkspaceError::DuplicatePanel(panel.id()));
        }
        if let Some((theme, backdrop)) = self.appearance {
            panel.set_theme(theme);
            panel.set_backdrop(backdrop);
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
        for group in &mut self.tabs {
            let position = group.members.iter().position(|member| *member == id);
            group.members.retain(|member| *member != id);
            if group.active == id && !group.members.is_empty() {
                group.active = group.members[position.unwrap_or(0).min(group.members.len() - 1)];
            }
        }
        self.tabs.retain(|group| group.members.len() > 1);
        Some(self.panels.remove(index))
    }

    #[must_use]
    pub fn tab_groups(&self) -> &[PaneTabs] { &self.tabs }

    #[must_use]
    pub fn tab_group(&self, id: PanelId) -> Option<&PaneTabs> {
        self.tabs.iter().find(|group| group.members.contains(&id))
    }

    #[must_use]
    pub fn tab_visible(&self, id: PanelId) -> bool {
        self.tab_group(id).is_none_or(|group| group.active == id)
    }

    /// Replaces tab groups only after validating every content reference.
    /// # Errors
    /// Rejects duplicate, missing, non-desktop, or inactive-member references.
    pub fn set_tab_groups(&mut self, groups: Vec<PaneTabs>) -> Result<(), WorkspaceError> {
        let mut seen = HashSet::new();
        if groups.iter().any(|group| group.members.len() < 2
            || !group.members.contains(&group.active)
            || group.members.iter().any(|id| !seen.insert(*id)
                || self.panel(*id).is_none_or(|panel| !panel.supports_tabs()))) {
            return Err(WorkspaceError::InvalidTabs);
        }
        self.tabs = groups;
        self.sync_tab_windows();
        Ok(())
    }

    /// Propagates window preferences from the active content to its siblings.
    pub fn sync_tab_windows(&mut self) {
        for group in self.tabs.clone() {
            let Some(source) = self.panel(group.active).cloned() else { continue; };
            for id in group.members {
                if let Some(panel) = self.panel_mut(id) {
                    panel.set_rect(source.rect());
                    panel.set_collapsed(source.collapsed());
                    panel.set_locked(source.locked());
                    panel.set_auto_hide(source.auto_hide());
                    panel.set_always_on_top(source.always_on_top());
                    panel.set_theme(source.theme());
                    panel.set_backdrop(source.backdrop());
                }
            }
        }
    }

    #[must_use]
    pub const fn appearance(&self) -> Option<(PanelTheme, Backdrop)> {
        self.appearance
    }

    /// Updates defaults for new panels without overwriting existing overrides.
    pub fn set_appearance_defaults(&mut self, theme: PanelTheme, backdrop: Backdrop) {
        self.appearance = Some((theme, backdrop));
    }

    pub fn set_appearance(&mut self, theme: PanelTheme, backdrop: Backdrop) {
        self.set_appearance_defaults(theme, backdrop);
        for panel in &mut self.panels {
            panel.set_theme(theme);
            panel.set_backdrop(backdrop);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceError {
    DuplicatePanel(PanelId),
    InvalidTabs,
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePanel(id) => write!(formatter, "panel {} already exists", id.get()),
            Self::InvalidTabs => formatter.write_str("invalid pane tab group"),
        }
    }
}

impl std::error::Error for WorkspaceError {}
