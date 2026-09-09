//! Native frames have a separate workspace: legacy takeover placements are never replayed.
use desktop_core::{Panel, PanelId, PanelSource, RectDip, Workspace};
use desktop_shell::move_native_desktop_items;
use desktop_storage::WorkspaceStore;
use desktop_window::{NativeFrame, NativeFrameEvent};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

struct NativeDesktop {
    workspace: Workspace,
    store: WorkspaceStore,
    frames: Vec<NativeFrame>,
}

pub fn run(path: &Path, title: Option<String>) -> Result<(), String> {
    let mut store = WorkspaceStore::open(path).map_err(|e| e.to_string())?;
    let mut workspace = store.load_workspace().map_err(|e| e.to_string())?;
    if workspace.panels().is_empty() {
        workspace
            .add_panel(Panel::new(
                PanelId::new(1),
                "新建分组",
                PanelSource::DesktopCollection,
                RectDip::default(),
            ))
            .map_err(|e| e.to_string())?;
    }
    let panels = workspace.panels().to_vec();
    if let Some(title) = title {
        workspace
            .panel_mut(panels[0].id())
            .unwrap()
            .set_title(title);
    }
    store
        .save_workspace(&workspace)
        .map_err(|e| e.to_string())?;
    let state = Rc::new(RefCell::new(NativeDesktop {
        workspace,
        store,
        frames: Vec::new(),
    }));
    for panel in panels {
        create_frame(&state, panel.id())?;
    }
    NativeFrame::run();
    Ok(())
}

fn create_frame(state: &Rc<RefCell<NativeDesktop>>, id: PanelId) -> Result<(), String> {
    let panel = state
        .borrow()
        .workspace
        .panel(id)
        .cloned()
        .ok_or("分组不存在")?;
    let weak = Rc::downgrade(state);
    let frame = NativeFrame::new(panel.title().to_string(), panel.rect(), move |event| {
        let Some(state) = weak.upgrade() else {
            return false;
        };
        if let Err(error) = handle_event(&state, id, event) {
            NativeFrame::show_error(&error);
            return false;
        }
        true
    })?;
    state.borrow_mut().frames.push(frame);
    Ok(())
}

#[allow(clippy::cast_possible_truncation)]
fn handle_event(
    state: &Rc<RefCell<NativeDesktop>>,
    id: PanelId,
    event: NativeFrameEvent,
) -> Result<(), String> {
    if matches!(event, NativeFrameEvent::NewFrame) {
        let next_id = {
            let mut state = state.borrow_mut();
            let next_id = PanelId::new(
                state
                    .workspace
                    .panels()
                    .iter()
                    .map(|p| p.id().get())
                    .max()
                    .unwrap_or(0)
                    + 1,
            );
            let base = state.workspace.panel(id).ok_or("分组不存在")?.rect();
            state
                .workspace
                .add_panel(Panel::new(
                    next_id,
                    "新建分组",
                    PanelSource::DesktopCollection,
                    RectDip {
                        x: base.x + 40.0,
                        y: base.y + 40.0,
                        ..RectDip::default()
                    },
                ))
                .map_err(|e| e.to_string())?;
            next_id
        };
        if let Err(error) = create_frame(state, next_id) {
            state.borrow_mut().workspace.remove_panel(next_id);
            return Err(error);
        }
    } else {
        let mut state = state.borrow_mut();
        match event {
            NativeFrameEvent::GeometryPreview { .. } | NativeFrameEvent::MenuRequested { .. } | NativeFrameEvent::MaterialChanged(_) => return Ok(()),
            NativeFrameEvent::GeometryChanged {
                previous,
                current,
                move_items,
            } => {
                if move_items {
                    move_native_desktop_items(
                        NativeFrame::content_bounds(previous),
                        (current.x - previous.x).round() as i32,
                        (current.y - previous.y).round() as i32,
                    )?;
                }
                if let Some(panel) = state.workspace.panel_mut(id) {
                    panel.set_rect(current);
                }
            }
            NativeFrameEvent::TitleChanged(title) => {
                if let Some(panel) = state.workspace.panel_mut(id) {
                    panel.set_title(title);
                }
            }
            NativeFrameEvent::RemoveFrame => {
                state.workspace.remove_panel(id);
                if state.workspace.panels().is_empty() {
                    NativeFrame::quit();
                }
            }
            NativeFrameEvent::Exit => NativeFrame::quit(),
            NativeFrameEvent::NewFrame => unreachable!(),
        }
    }
    let NativeDesktop {
        workspace, store, ..
    } = &mut *state.borrow_mut();
    if let Err(error) = store.save_workspace(workspace) {
        // Explorer may already have accepted the icon move. Keep the frame aligned with it.
        NativeFrame::show_error(&format!("无法保存分组，下次启动可能恢复到旧位置：{error}"));
    }
    Ok(())
}
