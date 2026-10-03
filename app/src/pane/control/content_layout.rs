//! Content sizing uses the renderer's grid; layout is previewed before one commit.
use super::*;
use luciddesk_api::{Operation, SnapAlign, SnapSide};
use luciddesk_window::MonitorDescriptor;

mod arrangement;
mod fitting;
mod measurement;
mod snapping;
#[cfg(test)]
mod tests;

#[cfg(test)]
use measurement::{FolderSnapshot, size};
pub(super) use measurement::{FolderSnapshots, live_query, snapshots};
use measurement::{members, metrics, pixel_size};

struct LayoutContext<'a> {
    workspace: &'a Workspace,
    positions: &'a HashMap<PanelId, RectDip>,
    monitors: &'a [MonitorDescriptor],
    folders: &'a FolderSnapshots,
}

fn wire(id: PanelId, m: &MonitorDescriptor, x: f32, y: f32, width: f32, height: f32) -> Operation {
    let scale = m.dpi as f32 / 96.0;
    Operation::Geometry {
        pane_id: id.get().to_string(),
        monitor_id: m.id.as_str().into(),
        x: x / scale,
        y: y / scale,
        width: width / scale,
        height: height / scale,
    }
}
fn parse(raw: &str) -> Result<PanelId, String> {
    raw.parse::<u64>()
        .ok()
        .filter(|v| *v > 0)
        .map(PanelId::new)
        .ok_or("invalid panel ID".into())
}
fn current(
    w: &Workspace,
    id: PanelId,
    positions: &HashMap<PanelId, RectDip>,
    monitors: &[MonitorDescriptor],
) -> Result<RectDip, String> {
    if let Some(r) = positions.get(&id) {
        return Ok(*r);
    }
    let r = w.panel(id).ok_or("panel does not exist")?.rect();
    let scale = monitors.first().ok_or("no monitor available")?.dpi as f32 / 96.0;
    Ok(RectDip {
        x: r.x * scale,
        y: r.y * scale,
        width: r.width * scale,
        height: r.height * scale,
    })
}
fn overlaps(a: RectDip, b: RectDip) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}
#[cfg(test)]
fn expand(
    w: &Workspace,
    store: &WorkspaceStore,
    op: &Operation,
    monitors: &[MonitorDescriptor],
    pending: &HashMap<PanelId, RectDip>,
) -> Result<Vec<Operation>, String> {
    expand_with_folders(w, store, op, monitors, pending, &HashMap::new())
}
pub(super) fn expand_with_folders(
    w: &Workspace,
    store: &WorkspaceStore,
    op: &Operation,
    monitors: &[MonitorDescriptor],
    pending: &HashMap<PanelId, RectDip>,
    folders: &FolderSnapshots,
) -> Result<Vec<Operation>, String> {
    let mut positions: HashMap<_, _> = store
        .monitor_layout(&display_layout::key(monitors))
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    positions.extend(pending.iter().map(|(k, v)| (*k, *v)));
    let context = LayoutContext {
        workspace: w,
        positions: &positions,
        monitors,
        folders,
    };
    match op {
        Operation::Snap {
            pane_id,
            target_pane_id,
            side,
            align,
            icon_columns,
        } => snapping::expand(&context, pane_id, target_pane_id, side, align, icon_columns),
        Operation::Fit { pane_id, .. } | Operation::FolderFit { pane_id, .. } => {
            fitting::expand(&context, op, pane_id)
        }
        Operation::Arrange {
            monitor_id,
            columns,
            icon_columns,
        } => arrangement::expand(&context, monitor_id, columns, icon_columns),
        _ => Err("not a content layout operation".into()),
    }
}
