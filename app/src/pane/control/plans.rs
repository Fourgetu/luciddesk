//! Validate on a workspace copy, then commit once. No window or Shell calls here.
use super::*;
use luciddesk_api::{Context, Operation, Plan};
use std::collections::{HashSet, VecDeque};
const TTL: Duration = Duration::from_secs(300);
pub(super) struct Prepared {
    token: String,
    base: Context,
    expires: Instant,
    next: Workspace,
    refs: HashMap<String, String>,
    diff: serde_json::Value,
    membership: bool,
    applied: bool,
}
struct Receipt {
    id: String,
    signature: String,
    response: Response,
    expires: Instant,
}
#[derive(Default)]
pub(super) struct Plans {
    pending: VecDeque<Prepared>,
    receipts: VecDeque<Receipt>,
}
fn invalid(message: impl Into<String>) -> (String, String) {
    ("INVALID_REQUEST".into(), message.into())
}
fn id(raw: &str) -> Result<PanelId, (String, String)> {
    raw.parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && *n <= i64::MAX as u64)
        .map(PanelId::new)
        .ok_or_else(|| invalid("invalid panel ID"))
}
fn editable(workspace: &Workspace, id: PanelId) -> Result<(), (String, String)> {
    let panel = workspace
        .panel(id)
        .ok_or_else(|| ("NOT_FOUND".into(), "panel does not exist".into()))?;
    if !panel.supports_tabs() {
        return Err(invalid("only desktop panels are supported"));
    }
    if panel.locked() {
        return Err(("PANE_LOCKED".into(), "panel is locked".into()));
    }
    Ok(())
}
fn ordered(workspace: &Workspace, pane: PanelId) -> Vec<String> {
    let mut items: Vec<_> = workspace
        .desktop_items()
        .iter()
        .filter_map(|item| match item.placement() {
            DesktopPlacement::Pane { pane_id, position } if *pane_id == pane => Some((
                (position.row, position.column),
                item.identity().persistent_key(),
            )),
            _ => None,
        })
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    items.into_iter().map(|(_, key)| key).collect()
}
fn order(workspace: &mut Workspace, pane: PanelId, keys: &[String]) {
    for (at, key) in keys.iter().enumerate() {
        if let Some(item) = workspace
            .desktop_items_mut()
            .iter_mut()
            .find(|i| i.identity().persistent_key() == *key)
        {
            item.set_placement(DesktopPlacement::Pane {
                pane_id: pane,
                position: GridPosition::new(at as u32, 0),
            });
        }
    }
}
fn keys(
    workspace: &Workspace,
    tokens: &[String],
    ids: &HashMap<String, String>,
) -> Result<Vec<String>, (String, String)> {
    if tokens.is_empty() || tokens.len() > 10000 {
        return Err(invalid("expected 1..10000 items"));
    }
    let mut seen = HashSet::new();
    tokens
        .iter()
        .map(|token| {
            if !seen.insert(token) {
                return Err(invalid("duplicate item ID"));
            }
            ids.iter()
                .find(|(_, v)| *v == token)
                .map(|(key, _)| key.clone())
                .filter(|key| {
                    workspace
                        .desktop_items()
                        .iter()
                        .any(|i| i.identity().persistent_key() == *key)
                })
                .ok_or_else(|| {
                    (
                        "NOT_FOUND".into(),
                        "item ID expired or does not exist; query workspace again".into(),
                    )
                })
        })
        .collect()
}
fn release(workspace: &mut Workspace, moving: &[String]) -> Result<(), (String, String)> {
    let moving: HashSet<_> = moving.iter().collect();
    let mut sources = HashSet::new();
    for item in workspace.desktop_items() {
        if moving.contains(&item.identity().persistent_key()) {
            if let DesktopPlacement::Pane { pane_id, .. } = item.placement() {
                editable(workspace, *pane_id)?;
                sources.insert(*pane_id);
            }
        }
    }
    for item in workspace.desktop_items_mut() {
        if moving.contains(&item.identity().persistent_key())
            && matches!(item.placement(), DesktopPlacement::Pane { .. })
        {
            item.set_placement(DesktopPlacement::default());
        }
    }
    for pane in sources {
        let items = ordered(workspace, pane);
        order(workspace, pane, &items);
    }
    Ok(())
}
fn summary(workspace: &Workspace) -> serde_json::Value {
    json!({"panes":workspace.panels().iter().map(|p|json!({"id":p.id().get().to_string(),"title":p.title(),"locked":p.locked(),"auto_hide":p.auto_hide(),"manual_collapsed":p.collapsed(),"always_on_top":p.always_on_top()})).collect::<Vec<_>>(),
        "placements":workspace.desktop_items().iter().map(|item|{
            let placement=match item.placement(){DesktopPlacement::Pane{pane_id,position}=>json!({"pane_id":pane_id.get().to_string(),"column":position.column,"row":position.row}),_=>serde_json::Value::Null};
            (item.identity().persistent_key(),placement)
        }).collect::<std::collections::BTreeMap<_,_>>()})
}
fn prepare(
    workspace: &Workspace,
    plan: &Plan,
    ids: &HashMap<String, String>,
) -> Result<Prepared, (String, String)> {
    if plan.protocol_version != 1 {
        return Err(("PROTOCOL_MISMATCH".into(), "plan protocol must be 1".into()));
    }
    if plan.operations.is_empty() || plan.operations.len() > 256 {
        return Err(invalid("expected 1..256 operations"));
    }
    let mut next = workspace.clone();
    let mut refs = HashMap::new();
    let mut membership = false;
    let mut next_id = workspace
        .panels()
        .iter()
        .map(|p| p.id().get())
        .max()
        .unwrap_or(0);
    for operation in &plan.operations {
        match operation {
            Operation::Create { reference, title } => {
                if reference.is_empty() || reference.len() > 128 || refs.contains_key(reference) {
                    return Err(invalid("invalid or duplicate panel ref"));
                }
                if title.trim().is_empty() || title.chars().count() > 256 {
                    return Err(invalid("title must contain 1..256 characters"));
                }
                next_id = next_id
                    .checked_add(1)
                    .filter(|n| *n <= i64::MAX as u64)
                    .ok_or_else(|| invalid("panel ID exhausted"))?;
                let number = next_id;
                let panel = Panel::new(
                    PanelId::new(number),
                    title,
                    display_layout::new_pane(&next, false),
                );
                next.add_panel(panel).map_err(|e| invalid(e.to_string()))?;
                refs.insert(reference.clone(), number.to_string());
            }
            Operation::Update {
                pane_id,
                title,
                locked,
                auto_hide,
                collapsed,
                always_on_top,
            } => {
                let target = id(pane_id)?;
                let panel = next
                    .panel(target)
                    .ok_or_else(|| ("NOT_FOUND".into(), "panel does not exist".into()))?;
                if !panel.supports_tabs() {
                    return Err(invalid("only desktop panels are currently supported"));
                }
                if title.is_none()
                    && locked.is_none()
                    && auto_hide.is_none()
                    && collapsed.is_none()
                    && always_on_top.is_none()
                {
                    return Err(invalid("update requires at least one field"));
                }
                if panel.locked() && *locked != Some(false) {
                    return Err((
                        "PANE_LOCKED".into(),
                        "explicitly unlock the panel first".into(),
                    ));
                }
                if let Some(title) = title {
                    if title.trim().is_empty() || title.chars().count() > 256 {
                        return Err(invalid("title must contain 1..256 characters"));
                    }
                    next.panel_mut(target).unwrap().set_title(title);
                }
                // Window options belong to every member of a tab group, including inactive tabs.
                let members = next
                    .tab_group(target)
                    .map_or_else(|| vec![target], |g| g.members.clone());
                for member in members {
                    let panel = next.panel_mut(member).unwrap();
                    if let Some(value) = locked {
                        panel.set_locked(*value);
                    }
                    if let Some(value) = auto_hide {
                        panel.set_auto_hide(*value);
                    }
                    if let Some(value) = collapsed {
                        panel.set_collapsed(*value);
                    }
                    if let Some(value) = always_on_top {
                        panel.set_always_on_top(*value);
                    }
                }
            }
            Operation::Release { item_ids } => {
                let moving = keys(&next, item_ids, ids)?;
                release(&mut next, &moving)?;
                membership = true;
            }
            Operation::Remove {
                pane_id,
                release_items,
            } => {
                let target = id(pane_id)?;
                editable(&next, target)?;
                let members = ordered(&next, target);
                if !members.is_empty() && !release_items {
                    return Err(invalid("panel is not empty; set release_items explicitly"));
                }
                if !members.is_empty() {
                    release(&mut next, &members)?;
                    membership = true;
                }
                next.remove_panel(target);
            }
            Operation::Assign {
                item_ids,
                pane_id,
                pane_ref,
            } => {
                membership = true;
                let target = match (pane_id, pane_ref) {
                    (Some(raw), None) => id(raw)?,
                    (None, Some(reference)) => id(refs
                        .get(reference)
                        .ok_or_else(|| invalid("unknown panel ref"))?)?,
                    _ => return Err(invalid("provide exactly one of pane_id/pane_ref")),
                };
                editable(&next, target)?;
                let moving = keys(&next, item_ids, ids)?;
                let mut sources = HashSet::new();
                for item in next.desktop_items() {
                    if moving.contains(&item.identity().persistent_key()) {
                        if let DesktopPlacement::Pane { pane_id, .. } = item.placement() {
                            editable(&next, *pane_id)?;
                            sources.insert(*pane_id);
                        }
                    }
                }
                let mut destination = ordered(&next, target);
                for key in moving {
                    if !destination.contains(&key) {
                        destination.push(key);
                    }
                }
                order(&mut next, target, &destination);
                for pane in sources {
                    let items = ordered(&next, pane);
                    order(&mut next, pane, &items);
                }
            }
            Operation::Reorder { pane_id, item_ids } => {
                membership = true;
                let target = id(pane_id)?;
                editable(&next, target)?;
                let requested = if item_ids.is_empty() {
                    Vec::new()
                } else {
                    keys(&next, item_ids, ids)?
                };
                let existing = ordered(&next, target);
                if requested.iter().collect::<HashSet<_>>()
                    != existing.iter().collect::<HashSet<_>>()
                {
                    return Err(invalid(
                        "reorder requires every current member exactly once",
                    ));
                }
                order(&mut next, target, &requested);
            }
        }
    }
    refs.retain(|_, raw| id(raw).is_ok_and(|id| next.panel(id).is_some()));
    let mut before = summary(workspace);
    let mut after = summary(&next);
    // Never expose internal Shell identity encoding in the wire diff.
    for value in [&mut before, &mut after] {
        let map = value["placements"].as_object_mut().unwrap();
        let old = std::mem::take(map);
        for (key, value) in old {
            if let Some(token) = ids.get(&key) {
                map.insert(token.clone(), value);
            }
        }
    }
    Ok(Prepared {
        token: luciddesk_api::request_id(),
        base: plan.base.clone(),
        expires: Instant::now() + TTL,
        next,
        refs,
        diff: json!({"before":before,"after":after}),
        membership,
        applied: false,
    })
}
impl Plans {
    pub(super) fn handle(
        &mut self,
        state: &mut PaneApp,
        context: &Context,
        ids: &HashMap<String, String>,
        request: &Request,
    ) -> Response {
        let fail = |code: &str, msg: &str| Response::failure(&request.request_id, code, msg);
        let now = Instant::now();
        self.pending.retain(|p| p.expires > now);
        self.receipts.retain(|r| r.expires > now);
        if request.command == "request.get" {
            return self.receipts.iter().find(|r|Some(&r.id)==request.id.as_ref()).map(|r|{
                Response::success(&request.request_id,json!(context),json!({"result":r.response}))
            }).unwrap_or_else(||fail("RESULT_UNKNOWN","request was not retained in this instance; inspect workspace before retrying"));
        }
        if request.command == "plan.preview" {
            let plan = request.plan.as_ref().unwrap();
            if plan.base != *context {
                return fail("CONFLICT", "workspace changed; query and preview again");
            }
            match prepare(&state.workspace, plan, ids) {
                Err((code, message)) => fail(&code, &message),
                Ok(plan) => {
                    let response = Response::success(
                        &request.request_id,
                        json!(context),
                        json!({"plan_token":plan.token,"expires_in_seconds":300,"changed":plan.next!=state.workspace,"diff":plan.diff,"provisional_refs":plan.refs}),
                    );
                    if serde_json::to_vec(&response)
                        .map_or(true, |bytes| bytes.len() > luciddesk_api::MAX_FRAME)
                    {
                        return fail("RESULT_TOO_LARGE", "preview exceeds response limit");
                    }
                    if self.pending.len() == 64 {
                        self.pending.pop_front();
                    }
                    self.pending.push_back(plan);
                    response
                }
            }
        } else {
            let signature = serde_json::to_string(request).unwrap();
            if let Some(receipt) = self.receipts.iter().find(|r| r.id == request.request_id) {
                return if receipt.signature == signature {
                    receipt.response.clone()
                } else {
                    fail("REQUEST_ID_REUSED", "request ID has different content")
                };
            }
            let Some(plan) = self
                .pending
                .iter_mut()
                .find(|p| Some(&p.token) == request.token.as_ref())
            else {
                return fail(
                    "PLAN_EXPIRED",
                    "plan expired or belongs to another instance",
                );
            };
            if plan.applied {
                return fail("PLAN_ALREADY_APPLIED", "plan has already been committed");
            }
            if plan.base != *context {
                return fail("CONFLICT", "workspace changed; preview again");
            }
            let changed = plan.next != state.workspace;
            if changed && plan.membership && !state.session.as_ref().is_some_and(hybrid::is_alive) {
                return fail(
                    "CAPABILITY_UNAVAILABLE",
                    "desktop integration must be connected for item operations",
                );
            }
            if changed {
                if let Err(error) = state.store.save_workspace(&plan.next) {
                    return fail("PERSISTENCE_ERROR", &error.to_string());
                }
                state.workspace = plan.next.clone();
            }
            plan.applied = true;
            let response = Response::success(
                &request.request_id,
                json!(context),
                json!({"changed":changed,"commit_status":if changed{"committed"}else{"unchanged"},"presentation_status":if changed{"pending"}else{"applied"},"refs":plan.refs}),
            );
            if self.receipts.len() == 1024 {
                self.receipts.pop_front();
            }
            self.receipts.push_back(Receipt {
                id: request.request_id.clone(),
                signature,
                response: response.clone(),
                expires: now + Duration::from_secs(600),
            });
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(command: &str) -> Request {
        serde_json::from_value(
            json!({"protocol_version":1,"request_id":luciddesk_api::request_id(),"command":command}),
        )
        .unwrap()
    }
    fn preview(
        snapshot: &mut Snapshot,
        state: &mut PaneApp,
        operations: serde_json::Value,
    ) -> Response {
        let context = snapshot
            .respond(state, &request("workspace.get"))
            .context
            .unwrap();
        let mut req = request("plan.preview");
        req.plan = Some(
            serde_json::from_value(
                json!({"protocol_version":1,"base":context,"operations":operations}),
            )
            .unwrap(),
        );
        snapshot.respond(state, &req)
    }
    fn apply(response: &Response) -> Request {
        assert!(response.ok, "{response:?}");
        let mut req = request("plan.apply");
        req.token = Some(
            response.data.as_ref().unwrap()["plan_token"]
                .as_str()
                .unwrap()
                .into(),
        );
        req
    }
    #[test]
    fn preview_is_pure_commit_is_idempotent_and_receipt_is_queryable() {
        let mut state = super::super::super::tests::test_state();
        state.store.save_workspace(&state.workspace).unwrap();
        let before = state.workspace.clone();
        let count = state.store.change_count();
        let mut snapshot = Snapshot::new();
        let p = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.create","ref":"new","title":"整理"},{"op":"pane.update","pane_id":"1","title":"文档"}]),
        );
        assert_eq!(state.workspace, before);
        assert_eq!(state.store.change_count(), count);
        let mut req = apply(&p);
        let response = snapshot.respond(&mut state, &req);
        assert!(response.ok, "{response:?}");
        assert_eq!(state.workspace.panels().len(), 3);
        assert_eq!(
            state.workspace.panel(PanelId::new(1)).unwrap().title(),
            "文档"
        );
        let committed = state.store.change_count();
        assert!(committed > count);
        let retry = snapshot.respond(&mut state, &req);
        assert_eq!(
            serde_json::to_value(&retry).unwrap(),
            serde_json::to_value(&response).unwrap()
        );
        assert_eq!(state.store.change_count(), committed);
        let mut query = request("request.get");
        query.id = Some(req.request_id.clone());
        assert!(snapshot.respond(&mut state, &query).ok);
        req.token = Some("different".into());
        assert_eq!(
            snapshot.respond(&mut state, &req).error.unwrap().code,
            "REQUEST_ID_REUSED"
        );
        assert_eq!(state.store.change_count(), committed);
    }
    #[test]
    fn expired_plan_and_evicted_receipt_are_reported_without_writes() {
        let mut state = super::super::super::tests::test_state();
        let mut snapshot = Snapshot::new();
        let p = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.create","ref":"new","title":"new"}]),
        );
        let req = apply(&p);
        let count = state.store.change_count();
        snapshot.plans.pending[0].expires = Instant::now() - Duration::from_secs(1);
        assert_eq!(
            snapshot.respond(&mut state, &req).error.unwrap().code,
            "PLAN_EXPIRED"
        );
        let mut query = request("request.get");
        query.id = Some(req.request_id);
        assert_eq!(
            snapshot.respond(&mut state, &query).error.unwrap().code,
            "RESULT_UNKNOWN"
        );
        assert_eq!(state.store.change_count(), count);
    }
    #[test]
    fn remove_requires_explicit_release_and_normalizes_only_affected_members() {
        let mut state = super::super::super::tests::test_state();
        let mut snapshot = Snapshot::new();
        for (index, item) in state.workspace.desktop_items_mut().iter_mut().enumerate() {
            item.set_placement(DesktopPlacement::Pane {
                pane_id: PanelId::new(1),
                position: GridPosition::new(index as u32, 0),
            });
        }
        let before = state.workspace.clone();
        let count = state.store.change_count();
        let denied = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.remove","pane_id":"1"}]),
        );
        assert!(!denied.ok);
        let prepared = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.remove","pane_id":"1","release_items":true}]),
        );
        assert!(prepared.ok);
        let next = &snapshot.plans.pending.back().unwrap().next;
        assert!(next.panel(PanelId::new(1)).is_none());
        assert!(
            next.desktop_items()
                .iter()
                .all(|i| matches!(i.placement(), DesktopPlacement::FreeDesktop { .. }))
        );
        assert_eq!(state.workspace, before);
        assert_eq!(state.store.change_count(), count);
        assert_eq!(
            snapshot
                .respond(&mut state, &apply(&prepared))
                .error
                .unwrap()
                .code,
            "CAPABILITY_UNAVAILABLE"
        );
    }
    #[test]
    fn options_require_explicit_unlock_and_preview_lists_shared_changes() {
        let mut state = super::super::super::tests::test_state();
        let mut snapshot = Snapshot::new();
        state
            .workspace
            .set_tab_groups(vec![luciddesk_core::PaneTabs {
                members: vec![PanelId::new(1), PanelId::new(2)],
                active: PanelId::new(1),
            }])
            .unwrap();
        let p = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.update","pane_id":"2","locked":true,"auto_hide":true,"collapsed":true,"always_on_top":true}]),
        );
        assert!(snapshot.respond(&mut state, &apply(&p)).ok);
        for panel in state.workspace.panels() {
            assert!(
                panel.locked() && panel.auto_hide() && panel.collapsed() && panel.always_on_top()
            );
        }
        let denied = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.update","pane_id":"1","title":"blocked"}]),
        );
        assert_eq!(denied.error.unwrap().code, "PANE_LOCKED");
        let p = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.update","pane_id":"2","locked":false,"title":"unlocked"}]),
        );
        assert!(snapshot.respond(&mut state, &apply(&p)).ok);
        assert!(state.workspace.panels().iter().all(|p| !p.locked()));
    }
    #[test]
    fn provisional_ids_are_not_reused_within_a_plan() {
        let mut state = super::super::super::tests::test_state();
        let mut snapshot = Snapshot::new();
        let p = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.create","ref":"discarded","title":"discarded"},{"op":"pane.remove","pane_id":"3"},{"op":"pane.create","ref":"kept","title":"kept"}]),
        );
        assert!(p.ok);
        let refs = &p.data.unwrap()["provisional_refs"];
        assert!(refs.get("discarded").is_none());
        assert_eq!(refs["kept"], "4");
    }
    #[test]
    fn stale_plan_and_invalid_batch_never_write() {
        let mut state = super::super::super::tests::test_state();
        let mut snapshot = Snapshot::new();
        let count = state.store.change_count();
        let original = state.workspace.clone();
        let invalid = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.create","ref":"new","title":"test"},{"op":"pane.update","pane_id":"999","title":"missing"}]),
        );
        assert!(!invalid.ok);
        assert_eq!(state.workspace, original);
        let p = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.update","pane_id":"1","title":"CLI"}]),
        );
        state
            .workspace
            .panel_mut(PanelId::new(1))
            .unwrap()
            .set_title("GUI");
        assert_eq!(
            snapshot.respond(&mut state, &apply(&p)).error.unwrap().code,
            "CONFLICT"
        );
        assert_eq!(state.store.change_count(), count);
    }
    #[test]
    fn no_op_and_disconnected_membership_do_not_write() {
        let mut state = super::super::super::tests::test_state();
        let mut snapshot = Snapshot::new();
        let count = state.store.change_count();
        let p = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.update","pane_id":"1","title":"Group 1"}]),
        );
        assert_eq!(
            snapshot.respond(&mut state, &apply(&p)).data.unwrap()["changed"],
            false
        );
        let items = snapshot
            .respond(&mut state, &request("workspace.get"))
            .data
            .unwrap()["items"]
            .clone();
        let p = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"item.assign","pane_id":"2","item_ids":[items[0]["id"]]}]),
        );
        assert_eq!(
            snapshot.respond(&mut state, &apply(&p)).error.unwrap().code,
            "CAPABILITY_UNAVAILABLE"
        );
        assert_eq!(state.store.change_count(), count);
    }
    #[test]
    fn assignment_preserves_unrelated_locked_pane_and_reorder_checks_membership() {
        let mut state = super::super::super::tests::test_state();
        let mut snapshot = Snapshot::new();
        let pane = PanelId::new(2);
        state.workspace.desktop_items_mut()[2].set_placement(DesktopPlacement::Pane {
            pane_id: pane,
            position: GridPosition::new(9, 3),
        });
        state.workspace.panel_mut(pane).unwrap().set_locked(true);
        let original = state.workspace.clone();
        let data = snapshot.respond(&mut state, &request("workspace.get"));
        let items = &data.data.as_ref().unwrap()["items"];
        let plan:Plan=serde_json::from_value(json!({"protocol_version":1,"base":data.context,"operations":[{"op":"pane.create","ref":"new","title":"new"},{"op":"item.assign","pane_ref":"new","item_ids":[items[0]["id"]]}]})).unwrap();
        let prepared = prepare(&state.workspace, &plan, &snapshot.item_ids).unwrap();
        assert_eq!(
            prepared.next.desktop_items()[2],
            original.desktop_items()[2]
        );
        assert_eq!(state.workspace, original);
        let p = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"item.reorder","pane_id":"1","item_ids":[items[0]["id"],items[0]["id"]]}]),
        );
        assert!(!p.ok);
        let p = preview(
            &mut snapshot,
            &mut state,
            json!([{"op":"pane.update","pane_id":"2","title":"locked"}]),
        );
        assert_eq!(p.error.unwrap().code, "PANE_LOCKED");
    }
}
