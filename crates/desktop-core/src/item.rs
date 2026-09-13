//! Desktop items and their workspace placement.
use crate::{GridPosition, MonitorId, PanelId, PointDip, ShellIdentity};

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
