//! Single operations reuse the server's preview, concurrency and receipt protocol.
use super::*;
use luciddesk_api::{Context, Operation, Plan};
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub(super) fn supported(command: &str) -> bool {
    matches!(
        command,
        "pane.create"
            | "pane.update"
            | "pane.remove"
            | "pane.geometry"
            | "folder.create"
            | "folder.update"
            | "folder.navigate"
            | "folder.back"
            | "folder.home"
            | "search.query"
            | "search.refresh"
            | "search.more"
            | "tab.merge"
            | "tab.select"
            | "tab.reorder"
            | "tab.detach"
            | "item.assign"
            | "item.release"
            | "item.reorder"
            | "settings.update"
            | "startup.set"
    )
}
pub(super) fn flag(flag: &str) -> Option<&'static str> {
    Some(match flag {
        "--title" => "title",
        "--path" => "path",
        "--query" => "query",
        "--into" => "into_pane_id",
        "--monitor" => "monitor_id",
        "--x" => "x",
        "--y" => "y",
        "--width" => "width",
        "--height" => "height",
        "--locked" => "locked",
        "--auto-hide" => "auto_hide",
        "--collapsed" => "collapsed",
        "--always-on-top" => "always_on_top",
        "--list-view" => "list_view",
        "--enabled" => "enabled",
        "--expected-status" => "expected_status",
        "--ids" => "item_ids",
        _ => return None,
    })
}
pub(super) fn value(flag_name: &str, raw: &str) -> Result<(String, Value), String> {
    let key = flag(flag_name).ok_or("unknown shortcut option")?;
    let value = match key {
        "locked" | "auto_hide" | "collapsed" | "always_on_top" | "list_view" | "enabled" => json!(
            raw.parse::<bool>()
                .map_err(|_| format!("{flag_name} expects true or false"))?
        ),
        "x" | "y" | "width" | "height" => {
            let n = raw
                .parse::<f64>()
                .map_err(|_| format!("{flag_name} expects a number"))?;
            if !n.is_finite() {
                return Err("non-finite geometry".into());
            }
            json!(n)
        }
        "item_ids" => {
            let ids: Vec<_> = raw.split(',').collect();
            if ids.iter().any(|id| id.is_empty()) {
                return Err("--ids expects comma-separated nonempty IDs".into());
            }
            json!(ids)
        }
        _ => json!(raw),
    };
    Ok((key.into(), value))
}
pub(super) fn operation(
    command: &str,
    id: Option<String>,
    pane: Option<String>,
    fields: BTreeMap<String, Value>,
    input: Option<Vec<u8>>,
) -> Result<Operation, String> {
    let mut object = serde_json::Map::new();
    if let Some(input) = input {
        let value: Value = serde_json::from_slice(&input).map_err(|e| e.to_string())?;
        if command == "settings.update" {
            object.insert("values".into(), value);
        } else if command == "item.reorder" && value.is_array() {
            object.insert("item_ids".into(), value);
        } else {
            object = value
                .as_object()
                .ok_or("shortcut input must be a JSON object")?
                .clone();
        }
    }
    let mut insert = |key: String, value: Value| -> Result<(), String> {
        if object.insert(key.clone(), value).is_some() {
            return Err(format!("duplicate operation field: {key}"));
        }
        Ok(())
    };
    for (key, value) in fields {
        insert(key, value)?;
    }
    if let Some(id) = id {
        insert("pane_id".into(), json!(id))?;
    }
    if let Some(pane) = pane {
        insert("pane_id".into(), json!(pane))?;
    }
    insert("op".into(), json!(command))?;
    if matches!(command, "pane.create" | "folder.create") && !object.contains_key("ref") {
        object.insert("ref".into(), json!("created"));
    }
    serde_json::from_value(Value::Object(object)).map_err(|e| e.to_string())
}
pub(super) fn run(
    options: &Options,
    mut call: impl FnMut(&Request, Duration) -> Response,
) -> Response {
    let Some(operation) = &options.operation else {
        return call(&options.request, options.timeout);
    };
    let mut query = options.request.clone();
    query.request_id = luciddesk_api::request_id();
    let snapshot = call(&query, options.timeout);
    if !snapshot.ok {
        return snapshot;
    }
    let context: Context = match snapshot
        .context
        .and_then(|v| serde_json::from_value(v).ok())
    {
        Some(context) => context,
        None => {
            return Response::failure(
                &options.request.request_id,
                "PROTOCOL_MISMATCH",
                "Workspace response omitted a valid context",
            );
        }
    };
    let mut preview = options.request.clone();
    preview.command = "plan.preview".into();
    preview.request_id = luciddesk_api::request_id();
    preview.plan = Some(Plan {
        protocol_version: options.request.protocol_version,
        base: context,
        operations: vec![operation.clone()],
    });
    let prepared = call(&preview, options.timeout);
    if !prepared.ok || options.dry_run {
        return prepared;
    }
    let Some(token) = prepared
        .data
        .as_ref()
        .and_then(|data| data["plan_token"].as_str())
        .map(str::to_owned)
    else {
        return Response::failure(
            &options.request.request_id,
            "PROTOCOL_MISMATCH",
            "Preview response omitted plan token",
        );
    };
    let mut apply = options.request.clone();
    apply.command = "plan.apply".into();
    apply.token = Some(token.clone());
    let mut response = call(&apply, options.timeout);
    // Even a transport failure must leave enough information for exact recovery.
    if response.data.is_none() {
        response.data = Some(json!({}));
    }
    response.data.as_mut().unwrap()["recovery"] =
        json!({"plan_token":token,"request_id":apply.request_id,"command":"plan.apply"});
    response
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parse_words(words: &[&str]) -> Options {
        parse(words.iter().map(OsString::from).collect()).unwrap()
    }
    #[test]
    fn shortcuts_preserve_typed_values_and_reject_ambiguous_flags() {
        let options = parse_words(&[
            "pane",
            "create",
            "--title",
            "中文 --title content",
            "--dry-run",
        ]);
        assert!(options.dry_run);
        assert!(
            matches!(options.operation,Some(Operation::Create{title,..}) if title=="中文 --title content")
        );
        let options = parse_words(&["pane", "update", "--id", "1", "--locked", "false"]);
        assert!(matches!(
            options.operation,
            Some(Operation::Update {
                locked: Some(false),
                ..
            })
        ));
        for words in [
            vec!["status", "--title", "bad"],
            vec!["pane", "create", "--id", "1", "--title", "bad"],
            vec!["pane", "update", "--id", "1", "--locked", "yes"],
            vec!["pane", "remove", "--id", "1", "--pane", "2"],
            vec!["status", "--dry-run"],
            vec!["pane", "geometry", "--x", "NaN"],
        ] {
            assert!(
                parse(words.iter().map(OsString::from).collect()).is_err(),
                "{words:?}"
            );
        }
    }
    #[test]
    fn shortcut_preview_and_apply_use_same_context_and_preserve_recovery_after_timeout() {
        for dry_run in [false, true] {
            let mut options =
                parse_words(&["pane", "remove", "--id", "1", "--request-id", "fixed-id"]);
            options.dry_run = dry_run;
            let context = json!({"instance_id":"instance","state_version":"1","inventory_version":"2","topology_token":"3"});
            let mut commands = Vec::new();
            let response = run(&options, |request, _| {
                commands.push(request.command.clone());
                match request.command.as_str() {
                    "workspace.get" => {
                        Response::success(&request.request_id, context.clone(), json!({}))
                    }
                    "plan.preview" => {
                        assert_eq!(json!(request.plan.as_ref().unwrap().base), context);
                        Response::success(
                            &request.request_id,
                            context.clone(),
                            json!({"plan_token":"token"}),
                        )
                    }
                    "plan.apply" => {
                        assert_eq!(request.token.as_deref(), Some("token"));
                        assert_eq!(request.request_id, "fixed-id");
                        Response::failure(&request.request_id, "TIMEOUT", "injected timeout")
                    }
                    _ => panic!("unexpected command"),
                }
            });
            if dry_run {
                assert_eq!(commands, vec!["workspace.get", "plan.preview"]);
                assert!(response.ok);
            } else {
                assert_eq!(
                    commands,
                    vec!["workspace.get", "plan.preview", "plan.apply"]
                );
                assert_eq!(response.exit_code(), 8);
                assert_eq!(response.data.unwrap()["recovery"]["plan_token"], "token");
            }
        }
    }
}
