//! Offline command discovery backed by the same protocol as execution.
use serde_json::{Value, json};
use std::ffi::OsString;

fn commands() -> Vec<&'static str> {
    luciddesk_api::COMMANDS
        .iter()
        .chain(luciddesk_api::OPERATIONS)
        .copied()
        .chain(["schema", "skill.show"])
        .collect()
}

fn document(topic: &str) -> Result<Value, String> {
    let commands = commands();
    let children: Vec<_> = commands
        .iter()
        .filter(|name| topic.is_empty() || name.starts_with(&format!("{topic}.")))
        .map(|name| name.replace('.', " "))
        .collect();
    if !topic.is_empty() && !commands.contains(&topic) && children.is_empty() {
        return Err(format!(
            "Unknown help topic '{topic}'. Run 'luciddesk-cli help' to list commands."
        ));
    }
    let mutation = luciddesk_api::OPERATIONS.contains(&topic);
    let command = topic.replace('.', " ");
    let arguments = match topic {
        "plan.preview" => " --input FILE|-",
        "plan.apply" => " --token TOKEN --request-id ID",
        "pane.get" | "folder.get" | "search.get" | "request.get" => " --id ID",
        "item.list" => " [--pane ID | --unassigned]",
        _ if mutation => " [--input FILE|-] [FIELD_FLAGS] [--dry-run]",
        _ if !children.is_empty() => " <command>",
        _ => "",
    };
    let mut doc = json!({
        "topic": if topic.is_empty() {"overview"} else {topic},
        "usage": format!("luciddesk-cli {command}{arguments} [--json]"),
        "commands": children,
        "options": ["--json: machine-readable output", "--data-dir PATH: assert the running app's directory", "--timeout-ms 1..60000: default 10000", "--protocol-version N: protocol compatibility", "--request-id ID: retain for recovery", "help [RESOURCE [COMMAND]], -h, --help: offline help", "--version: CLI and protocol version"],
        "notes": ["Online commands require the matching GUI to be running. The CLI never opens the database.", "Help, schema and skill show are offline; help accepts only a topic and optional --json.", "Workflow: query -> preview -> apply -> verify. Shortcut mutations preview then apply unless --dry-run is present.", "On timeout, query request get with the original request ID before retrying. Do not repeat an unknown mutation with a new ID."]
    });
    if mutation {
        let schema: Value = serde_json::from_str(include_str!(
            "../../crates/luciddesk-api/protocol.schema.json"
        ))
        .unwrap();
        fn find<'a>(v: &'a Value, topic: &str) -> Option<&'a Value> {
            if v.pointer("/properties/op/const").and_then(Value::as_str) == Some(topic) {
                return Some(v);
            }
            match v {
                Value::Object(map) => map.values().find_map(|v| find(v, topic)),
                Value::Array(values) => values.iter().find_map(|v| find(v, topic)),
                _ => None,
            }
        }
        doc["operation_schema"] = find(&schema, topic)
            .ok_or_else(|| format!("Missing schema for {topic}"))?
            .clone();
        doc["input"] = json!(match topic {
            "settings.update" => "--input contains the plain settings map (not a values wrapper).",
            "item.reorder" =>
                "--input accepts a complete item-ID array or an operation-fields object without op.",
            _ =>
                "--input contains an operation-fields object without op; use UTF-8 without BOM. FILE=- reads stdin. Do not duplicate fields in input and flags. Creation ref defaults to created.",
        });
        let flags = [
            "--target",
            "--descending",
            "--side",
            "--align",
            "--max-rows",
            "--icon-columns",
            "--title",
            "--path",
            "--query",
            "--into",
            "--monitor",
            "--x",
            "--y",
            "--width",
            "--height",
            "--locked",
            "--auto-hide",
            "--collapsed",
            "--always-on-top",
            "--list-view",
            "--enabled",
            "--expected-status",
            "--ids",
        ];
        let properties = doc["operation_schema"]["properties"].as_object().unwrap();
        let mut supported: Vec<_> = flags
            .iter()
            .filter_map(|flag| {
                let field = crate::shortcuts::flag(flag)?;
                properties
                    .contains_key(field)
                    .then(|| format!("{flag} VALUE -> {field}"))
            })
            .collect();
        if properties.contains_key("pane_id") {
            supported.push("--id ID (or --pane ID) -> pane_id".into());
        }
        if properties.contains_key("release_items") {
            supported.push("--release-items -> release_items:true".into());
        }
        doc["field_flags"] = json!(supported);
        doc["notes"].as_array_mut().unwrap().push(json!("Fields can be provided using flags, --input, or both without duplicates. Schema required fields apply to the combined payload. Settings, startup and transient actions require separate plans."));
    }
    Ok(doc)
}

pub(super) fn output(args: &[OsString]) -> Option<Result<String, String>> {
    let requested = args.is_empty()
        || args.first().is_some_and(|a| a == "help")
        || args.iter().any(|a| a == "--help" || a == "-h");
    if !requested {
        return None;
    }
    Some((|| {
        let mut words = Vec::new();
        let mut json = false;
        let mut help = false;
        for (index, arg) in args.iter().enumerate() {
            let arg = arg.to_str().ok_or("Help arguments must be valid Unicode")?;
            match arg {
                "--json" if !json => json = true,
                "help" if index == 0 && !help => help = true,
                "--help" | "-h" if !help => help = true,
                option if option.starts_with('-') => {
                    return Err(format!(
                        "Unexpected help option '{option}'. Use 'luciddesk-cli help RESOURCE COMMAND [--json]'."
                    ));
                }
                word => words.push(word),
            }
        }
        let doc = document(&words.join("."))?;
        if json {
            return Ok(serde_json::to_string(&luciddesk_api::Response::success(
                "offline",
                Value::Null,
                doc,
            ))
            .unwrap());
        }
        let mut text = format!(
            "LucidDesk CLI\n\nUsage: {}\n",
            doc["usage"].as_str().unwrap()
        );
        for (key, label) in [
            ("commands", "Commands"),
            ("field_flags", "Field flags"),
            ("options", "Options"),
            ("notes", "Notes"),
        ] {
            if let Some(values) = doc[key].as_array().filter(|v| !v.is_empty()) {
                text.push_str(&format!("\n{label}:\n"));
                for value in values {
                    text.push_str(&format!("  {}\n", value.as_str().unwrap()));
                }
            }
        }
        if let Some(input) = doc["input"].as_str() {
            text.push_str(&format!("\nInput: {input}\n"));
        }
        if let Some(fields) = doc["operation_schema"]["properties"].as_object() {
            text.push_str("\nOperation fields (JSON constraints):\n");
            for (name, schema) in fields.iter().filter(|(name, _)| name.as_str() != "op") {
                let required = doc["operation_schema"]["required"]
                    .as_array()
                    .is_some_and(|v| v.contains(&json!(name)));
                text.push_str(&format!(
                    "  {name}{}: {schema}\n",
                    if required { " (required)" } else { "" }
                ));
            }
        }
        text.push_str("\nExamples: luciddesk-cli help pane snap\n          luciddesk-cli workspace get --json\n          luciddesk-cli pane fit --id 1 --icon-columns 6 --dry-run --json\n");
        Ok(text)
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(s: &str) -> Vec<OsString> {
        s.split_whitespace().map(Into::into).collect()
    }
    #[test]
    fn every_protocol_command_has_help_and_every_mutation_has_schema() {
        for name in commands() {
            let doc = document(name).unwrap();
            if luciddesk_api::OPERATIONS.contains(&name) {
                assert_eq!(doc["operation_schema"]["properties"]["op"]["const"], name);
            }
        }
    }
    #[test]
    fn help_aliases_are_offline_and_json_is_an_envelope() {
        let expected = output(&args("help pane snap")).unwrap().unwrap();
        for command in ["pane snap --help", "pane snap -h", "--help pane snap"] {
            assert_eq!(output(&args(command)).unwrap().unwrap(), expected);
        }
        let json: Value =
            serde_json::from_str(&output(&args("help pane snap --json")).unwrap().unwrap())
                .unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["data"]["topic"], "pane.snap");
        assert!(output(&args("pane snap --id 1")).is_none());
    }
    #[test]
    fn invalid_help_never_falls_through_to_execution() {
        for command in [
            "help nonsense",
            "help pane nonsense",
            "help --json --json",
            "pane snap --help --input -",
            "help --help",
        ] {
            assert!(output(&args(command)).unwrap().is_err(), "{command}");
        }
        assert!(output(&[]).unwrap().is_ok());
        assert!(
            !document("pane").unwrap()["commands"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
