use desktop_api::{Request, Response, VERSION};
use std::{ffi::OsString, time::Duration};
const HELP: &str = "LucidDesk CLI (read-only)\nUsage: luciddesk-cli <command> [options]\nCommands: schema (offline), status, capabilities, workspace get, pane list, pane get --id ID, item list\nOptions: --json, --data-dir PATH, --timeout-ms 1..60000, --protocol-version N\nItem filters: --pane ID | --unassigned\nThe GUI must already be running. This CLI never opens the database.";
struct Options {
    request: Request,
    json: bool,
    timeout: Duration,
}
fn parse(args: Vec<OsString>) -> Result<Options, String> {
    let mut request = Request {
        protocol_version: VERSION,
        request_id: desktop_api::request_id(),
        command: String::new(),
        id: None,
        pane: None,
        unassigned: false,
        data_dir: None,
    };
    let mut json = false;
    let mut timeout = 10000;
    let mut words = Vec::new();
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
            "--unassigned" => request.unassigned = true,
            "--id" | "--pane" | "--data-dir" | "--timeout-ms" | "--protocol-version" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} requires a value"))?
                    .into_string()
                    .map_err(|_| "invalid Unicode option")?;
                match arg.as_str() {
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
    request.validate().map_err(str::to_owned)?;
    Ok(Options {
        request,
        json,
        timeout: Duration::from_millis(timeout),
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
            include_str!("../../crates/desktop-api/protocol.schema.json")
        );
        return;
    }
    let wants_json = args.iter().any(|a| a == "--json");
    let (response, json) = match parse(args) {
        Err(error) => (Response::failure("", "INVALID_REQUEST", error), wants_json),
        Ok(options) => {
            let response = desktop_api::transport::call(&options.request, options.timeout)
                .unwrap_or_else(|e| {
                    let code = match e.kind() {
                        std::io::ErrorKind::NotFound => "APP_NOT_RUNNING",
                        std::io::ErrorKind::PermissionDenied => "ACCESS_DENIED",
                        std::io::ErrorKind::TimedOut => "TIMEOUT",
                        _ => "TRANSPORT_ERROR",
                    };
                    Response::failure(&options.request.request_id, code, e.to_string())
                });
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
        for s in [
            "pane get",
            "status --id 1",
            "item list --pane 1 --unassigned",
            "pane remove --id 1",
            "status --timeout-ms 0",
            "status --json --json",
        ] {
            assert!(parse(args(s)).is_err(), "{s}");
        }
    }
}
