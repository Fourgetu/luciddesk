mod shortcuts;
use luciddesk_api::{Request, Response, VERSION};
use std::{ffi::OsString, time::Duration};
const HELP: &str = "LucidDesk CLI\nUsage: luciddesk-cli <command> [options]\nCommands: schema (offline), skill show (offline), status, capabilities, workspace get, settings get, font list, startup get, monitor list, folder get --id ID, search get --id ID, pane list, pane get --id ID, item list, plan preview --input FILE|-, plan apply --token TOKEN --request-id ID, request get --id ID\nOptions: --json, --data-dir PATH, --timeout-ms 1..60000, --protocol-version N\nItem filters: --pane ID | --unassigned\nShortcut mutations: pane create/update/remove/geometry, folder create/update/navigate/back/home, search query/refresh/more, tab merge/select/reorder/detach, item assign/release/reorder, settings update, startup set. Add --dry-run to preview only; otherwise the CLI previews then applies once. Use --input FILE|- for operation fields or documented flags.
The GUI must already be running. This CLI never opens the database.";
struct Options {
    request: Request,
    json: bool,
    timeout: Duration,
    operation: Option<luciddesk_api::Operation>,
    dry_run: bool,
}
fn parse(args: Vec<OsString>) -> Result<Options, String> {
    let mut request = Request {
        protocol_version: VERSION,
        request_id: luciddesk_api::request_id(),
        command: String::new(),
        id: None,
        pane: None,
        unassigned: false,
        data_dir: None,
        plan: None,
        token: None,
    };
    let mut json = false;
    let mut timeout = 10000;
    let mut words = Vec::new();
    let mut input = None;
    let mut fields = std::collections::BTreeMap::new();
    let mut dry_run = false;
    let mut args = args.into_iter();
    let mut seen = std::collections::HashSet::new();
    while let Some(raw) = args.next() {
        let arg = raw
            .into_string()
            .map_err(|_| "arguments must be valid Unicode")?;
        if arg.starts_with('-') && !seen.insert(arg.clone()) {
            return Err(format!("duplicate option {arg}"));
        }
        match arg.as_str() {
            "--json" => json = true,
            "--dry-run" => dry_run = true,
            "--release-items" => {
                fields.insert("release_items".into(), serde_json::json!(true));
            }
            flag if shortcuts::flag(flag).is_some() => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{flag} requires a value"))?
                    .into_string()
                    .map_err(|_| "invalid Unicode option")?;
                let (key, value) = shortcuts::value(flag, &value)?;
                fields.insert(key, value);
            }
            "--unassigned" => request.unassigned = true,
            "--id" | "--pane" | "--data-dir" | "--timeout-ms" | "--protocol-version"
            | "--input" | "--token" | "--request-id" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} requires a value"))?
                    .into_string()
                    .map_err(|_| "invalid Unicode option")?;
                match arg.as_str() {
                    "--input" => input = Some(value),
                    "--token" => request.token = Some(value),
                    "--request-id" => request.request_id = value,
                    "--id" => request.id = Some(value),
                    "--pane" => request.pane = Some(value),
                    "--data-dir" => {
                        let path = std::fs::canonicalize(value)
                            .map_err(|e| format!("invalid data directory: {e}"))?;
                        request.data_dir = Some(path.to_string_lossy().into_owned());
                    }
                    "--timeout-ms" => {
                        timeout = value
                            .parse::<u64>()
                            .ok()
                            .filter(|n| (1..=60000).contains(n))
                            .ok_or("timeout must be 1..60000 ms")?
                    }
                    _ => {
                        request.protocol_version =
                            value.parse().map_err(|_| "invalid protocol version")?
                    }
                }
            }
            value if value.starts_with('-') => return Err(format!("unknown option {value}")),
            _ => words.push(arg),
        }
    }
    request.command = words.join(".");
    let operation = if shortcuts::supported(&request.command) {
        if request.token.is_some() || request.unassigned {
            return Err("shortcut commands reject --token and --unassigned".into());
        }
        let payload = input.map(|path| read_input(&path)).transpose()?;
        let operation = shortcuts::operation(
            &request.command,
            request.id.take(),
            request.pane.take(),
            fields,
            payload,
        )?;
        request.command = "workspace.get".into();
        Some(operation)
    } else {
        if dry_run || !fields.is_empty() {
            return Err("mutation options require a shortcut command".into());
        }
        if let Some(input) = input {
            if request.command != "plan.preview" {
                return Err("--input is only valid for plan preview or shortcut commands".into());
            }
            request.plan =
                Some(serde_json::from_slice(&read_input(&input)?).map_err(|e| e.to_string())?);
        }
        None
    };
    request.validate().map_err(str::to_owned)?;
    Ok(Options {
        request,
        json,
        timeout: Duration::from_millis(timeout),
        operation,
        dry_run,
    })
}
fn read_input(path: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut reader: Box<dyn Read> = if path == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(path).map_err(|e| e.to_string())?)
    };
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take((luciddesk_api::MAX_FRAME + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > luciddesk_api::MAX_FRAME {
        return Err("input exceeds 4 MiB".into());
    }
    luciddesk_api::validate_json(&bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}
fn call(request: &Request, timeout: Duration) -> Response {
    luciddesk_api::transport::call(request, timeout).unwrap_or_else(|e| {
        let code = match e.kind() {
            std::io::ErrorKind::NotFound => "APP_NOT_RUNNING",
            std::io::ErrorKind::PermissionDenied => "ACCESS_DENIED",
            std::io::ErrorKind::TimedOut => "TIMEOUT",
            _ => "TRANSPORT_ERROR",
        };
        Response::failure(&request.request_id, code, e.to_string())
    })
}
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args == [OsString::from("--help")] || args.is_empty() {
        println!("{HELP}");
        return;
    }
    if args == [OsString::from("--version")] {
        println!(
            "luciddesk-cli {} protocol {VERSION}",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    if args == [OsString::from("schema")]
        || args == [OsString::from("schema"), OsString::from("--json")]
    {
        println!(
            "{}",
            include_str!("../../crates/luciddesk-api/protocol.schema.json")
        );
        return;
    }
    if args == [OsString::from("skill"), OsString::from("show")]
        || args
            == [
                OsString::from("skill"),
                OsString::from("show"),
                OsString::from("--json"),
            ]
    {
        const SKILL: &str = include_str!("../../skills/luciddesk-control/SKILL.md");
        if args.last().is_some_and(|a| a == "--json") {
            let response = Response::success(
                "offline",
                serde_json::Value::Null,
                serde_json::json!({"name":"luciddesk-control","format":"markdown","content":SKILL}),
            );
            println!(
                "{}",
                serde_json::to_string(&response).expect("skill serialization")
            );
        } else {
            println!("{SKILL}");
        }
        return;
    }
    let wants_json = args.iter().any(|a| a == "--json");
    let (response, json) = match parse(args) {
        Err(error) => (Response::failure("", "INVALID_REQUEST", error), wants_json),
        Ok(options) => {
            let response = shortcuts::run(&options, call);
            (response, options.json)
        }
    };
    let code = response.exit_code();
    if json {
        println!(
            "{}",
            serde_json::to_string(&response).expect("response serialization")
        );
    } else if let Some(error) = &response.error {
        eprintln!("{}: {}", error.code, error.message);
        if let Some(recovery) = response.data.as_ref().and_then(|data| data.get("recovery")) {
            eprintln!(
                "Recovery: {}",
                serde_json::to_string(recovery).expect("recovery serialization")
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&response.data).expect("response serialization")
        );
    }
    std::process::exit(code);
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(s: &str) -> Vec<OsString> {
        s.split_whitespace().map(Into::into).collect()
    }
    #[test]
    fn options_validate_commands_and_filters() {
        assert!(parse(args("--json pane get --id 9007199254740993")).is_ok());
        assert!(parse(args("plan apply --token token --request-id retry-1 --json")).is_ok());
        assert!(parse(args("request get --id retry-1 --json")).is_ok());
        assert!(parse(args("settings get --json")).is_ok());
        assert!(parse(args("font list --json")).is_ok());
        assert!(parse(args("folder get --id 1 --json")).is_ok());
        assert!(parse(args("search get --id 1 --json")).is_ok());
        for s in [
            "pane get",
            "status --id 1",
            "item list --pane 1 --unassigned",
            "pane remove",
            "plan preview",
            "plan apply",
            "status --token token",
            "request get",
            "status --timeout-ms 0",
            "status --json --json",
        ] {
            assert!(parse(args(s)).is_err(), "{s}");
        }
    }
}
