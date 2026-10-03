//! Relative placement against an existing panel window.
use super::*;

pub(super) fn expand(
    context: &LayoutContext<'_>,
    pane_id: &str,
    target_pane_id: &str,
    side: &SnapSide,
    align: &SnapAlign,
    icon_columns: &Option<u32>,
) -> Result<Vec<Operation>, String> {
    let LayoutContext {
        workspace: w,
        positions,
        monitors,
        folders,
    } = *context;
    let mut result = Vec::new();
    let id = parse(pane_id)?;
    let target = parse(target_pane_id)?;
    let moving = members(w, id);
    let anchors = members(w, target);
    if moving.iter().any(|id| anchors.contains(id)) {
        return Err("cannot snap a panel to its own shared window".into());
    }
    for member in moving.iter().chain(anchors.iter()) {
        let p = w.panel(*member).ok_or("panel does not exist")?;
        if p.is_search() {
            return Err("relative snapping requires fixed-height desktop or folder panels; use geometry for search".into());
        }
        if p.collapsed() {
            return Err("expand both panels before relative snapping".into());
        }
    }
    let anchor = current(w, target, positions, monitors)?;
    let description = geometry::describe(anchor, monitors);
    let m = monitors
        .iter()
        .find(|m| Some(m.id.as_str()) == description["monitor_id"].as_str())
        .ok_or("target monitor unavailable")?;
    let scale = m.dpi as f32 / 96.0;
    let (width, height) = if let Some(columns) = icon_columns {
        pixel_size(w, id, *columns, scale, folders, None)?
    } else {
        let r = w.panel(id).unwrap().rect();
        ((r.width * scale).round(), (r.height * scale).round())
    };
    let gap = snap::GAP_PX as f32;
    let offset = |available: f32, extent: f32| match align {
        SnapAlign::Start => 0.0,
        SnapAlign::Center => ((available - extent) / 2.0).round(),
        SnapAlign::End => available - extent,
    };
    let (x, y) = match side {
        SnapSide::Left => (
            anchor.x - width - gap,
            anchor.y + offset(anchor.height, height),
        ),
        SnapSide::Right => (
            anchor.x + anchor.width + gap,
            anchor.y + offset(anchor.height, height),
        ),
        SnapSide::Top => (
            anchor.x + offset(anchor.width, width),
            anchor.y - height - gap,
        ),
        SnapSide::Bottom => (
            anchor.x + offset(anchor.width, width),
            anchor.y + anchor.height + gap,
        ),
    };
    let placed = RectDip {
        x,
        y,
        width,
        height,
    };
    for panel in w
        .panels()
        .iter()
        .filter(|p| !moving.contains(&p.id()) && !anchors.contains(&p.id()))
    {
        if overlaps(placed, current(w, panel.id(), positions, monitors)?) {
            return Err(format!(
                "snap would overlap panel {}; move it first",
                panel.id().get()
            ));
        }
    }
    let proposed = wire(
        id,
        m,
        x - m.work_area.x as f32,
        y - m.work_area.y as f32,
        width,
        height,
    );
    if let Operation::Geometry {
        x,
        y,
        width,
        height,
        ..
    } = &proposed
    {
        geometry::convert(
            m,
            RectDip {
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            },
        )?;
    }
    result.push(proposed);
    Ok(result)
}
