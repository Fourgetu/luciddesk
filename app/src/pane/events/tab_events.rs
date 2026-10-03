//! Tab lifecycle and drag-merge commands, before borrowing the app.
use super::*;

pub(super) fn route(
    state: &Rc<RefCell<PaneApp>>,
    id: PanelId,
    event: &Event,
) -> Result<Option<bool>, String> {
    match *event {
        Event::DetachTab(target) => {
            tabs::detach(state, target)?;
            return Ok(Some(false));
        }
        Event::PreviewPaneMove => {
            tabs::preview_merge(&state.borrow(), id);
            return Ok(Some(false));
        }
        Event::FinishPaneMove(commit) => {
            tabs::finish_move(state, id, commit)?;
            return Ok(Some(false));
        }
        Event::RenameTab(target) => {
            let (owner, model) = {
                let s = state.borrow();
                if s.workspace.panel(target).is_none_or(Panel::locked) {
                    return Ok(Some(false));
                }
                if !s
                    .workspace
                    .tab_group(id)
                    .is_some_and(|g| g.members.contains(&target))
                {
                    return Ok(Some(false));
                }
                let view = s
                    .views
                    .iter()
                    .find(|v| v.id == id)
                    .ok_or(crate::i18n::text("ui-panel-closed"))?;
                (view.window.hwnd().cast(), view.model.clone())
            };
            let state = Rc::clone(state);
            rename::show_tab_title(
                owner,
                model,
                Some(target),
                Box::new(move |title| handle(&state, target, Event::SetTitle(title)).map(|_| ())),
            )?;
            return Ok(Some(false));
        }
        Event::MoveTabId(target, step) => {
            let next = state.borrow().workspace.tab_group(id).and_then(|g| {
                let at = g.members.iter().position(|member| *member == target)?;
                g.members
                    .get(at.checked_add_signed(step as isize)?)
                    .copied()
            });
            if let Some(next) = next {
                tabs::reorder(state, id, target, next)?;
            }
            return Ok(Some(false));
        }
        Event::NewTab(false) => {
            tabs::add(state, id, None)?;
            return Ok(Some(false));
        }
        Event::NewTab(true) => {
            tabs::choose_folder(state, id)?;
            return Ok(Some(false));
        }
        Event::SelectTab(to) => {
            tabs::select(state, id, to)?;
            return Ok(Some(false));
        }
        Event::CloseTabId(to) => {
            tabs::close(state, to)?;
            return Ok(Some(false));
        }
        Event::CloseTab => {
            tabs::close(state, id)?;
            return Ok(Some(false));
        }
        Event::MoveTab(step) => {
            let destination = state.borrow().workspace.tab_group(id).and_then(|group| {
                let at = group.members.iter().position(|member| *member == id)?;
                let next = at.checked_add_signed(step as isize)?;
                group.members.get(next).copied()
            });
            if let Some(to) = destination {
                tabs::reorder(state, id, id, to)?;
            }
            return Ok(Some(false));
        }
        _ => {}
    }
    Ok(None)
}
