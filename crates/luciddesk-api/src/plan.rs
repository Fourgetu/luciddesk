//! Typed plans reject unknown operations and fields before domain execution.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub instance_id: String,
    pub state_version: String,
    pub inventory_version: String,
    pub topology_token: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub protocol_version: u32,
    pub base: Context,
    pub operations: Vec<Operation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
pub enum Operation {
    #[serde(rename = "pane.create")]
    Create {
        #[serde(rename = "ref")]
        reference: String,
        title: String,
    },
    #[serde(rename = "pane.update")]
    Update { pane_id: String, title: String },
    #[serde(rename = "item.assign")]
    Assign {
        item_ids: Vec<String>,
        #[serde(default)]
        pane_id: Option<String>,
        #[serde(default)]
        pane_ref: Option<String>,
    },
    #[serde(rename = "item.reorder")]
    Reorder {
        pane_id: String,
        item_ids: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operations_reject_unknown_fields_and_preserve_string_ids() {
        let raw = r#"{"op":"pane.update","pane_id":"9007199254740993","title":"整理"}"#;
        let op: Operation = serde_json::from_str(raw).unwrap();
        assert_eq!(
            serde_json::to_value(op).unwrap()["pane_id"],
            "9007199254740993"
        );
        assert!(serde_json::from_str::<Operation>(&raw.replace("title", "unknown")).is_err());
        assert!(
            serde_json::from_str::<Operation>(r#"{"op":"item.remove","item_ids":[]}"#).is_err()
        );
        assert!(
            serde_json::from_str::<Operation>(
                r#"{"op":"pane.update","pane_id":"1","title":"a","title":"b"}"#
            )
            .is_err()
        );
    }
}
