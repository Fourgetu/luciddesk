//! Complete offline skill bundle; plain output remains the entrypoint Markdown.
use serde_json::{Value, json};

pub(super) const ENTRYPOINT: &str = include_str!("../../skills/luciddesk-control/SKILL.md");

pub(super) fn bundle() -> Value {
    json!({
        "name":"luciddesk-control", "format":"markdown", "content":ENTRYPOINT,
        "bundle_version":1, "entrypoint":"SKILL.md",
        "files":{
            "SKILL.md":ENTRYPOINT,
            "references/plans-and-recovery.md":include_str!("../../skills/luciddesk-control/references/plans-and-recovery.md"),
            "references/desktop-layout.md":include_str!("../../skills/luciddesk-control/references/desktop-layout.md"),
            "references/folder-search.md":include_str!("../../skills/luciddesk-control/references/folder-search.md"),
            "references/settings-startup.md":include_str!("../../skills/luciddesk-control/references/settings-startup.md"),
            "references/installation.md":include_str!("../../skills/luciddesk-control/references/installation.md")
        }
    })
}
