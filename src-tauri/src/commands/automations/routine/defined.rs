//! A routine's step with `defines_item` turns its result into the item the later steps work on:
//! `{item.*}`, `WTM_ITEM_*`, the prompts' item and the tracker use it. The run log keeps it as
//! `item_defined`, so resumes, retries and the run detail find it; the item's events keep the
//! implicit item's key.

use serde_json::{Map, Value};

use super::super::definition::Step;
use super::super::selector::item_problem;
use super::super::store::events::{DefinedItem, LoggedEvent, RunEvent};
use super::super::wait::approved_params;
use super::commands::Item;

/// The item `result` defines; `version` is the implicit item's unless the result names one.
/// Agents whose results cannot hold free-form objects may give `data` as the text of one.
pub(super) fn defined_item(result: Option<&Value>, version: &str) -> Result<DefinedItem, String> {
    let result = result
        .and_then(Value::as_object)
        .ok_or("the result that defines the item is not a JSON object")?;
    let id = optional_text(result, "id")?.ok_or("the result that defines the item has no id")?;
    let version = optional_text(result, "version")?.unwrap_or_else(|| version.to_string());
    let title = optional_text(result, "title")?;
    let data = match result.get("data") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(data)) => data.clone(),
        Some(Value::String(text)) => serde_json::from_str(text)
            .map_err(|_| "the item's data is not the text of a JSON object".to_string())?,
        Some(_) => return Err("the item's data is not a JSON object".into()),
    };
    if let Some(problem) = item_problem(&id, &version, &data) {
        return Err(problem);
    }
    Ok(DefinedItem {
        id,
        version,
        title,
        data,
    })
}

fn optional_text(result: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match result.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("the item's {key} is not a string")),
    }
}

/// The item the latest `item_defined` of `key` in `history` defined, if any, and the data
/// approvers edited after it.
pub(super) fn logged_item(
    history: &[LoggedEvent],
    key: &str,
    steps: &[Step],
) -> (Option<Item>, Map<String, Value>) {
    let defined = history
        .iter()
        .enumerate()
        .rev()
        .find_map(|(position, logged)| match &logged.event {
            RunEvent::ItemDefined { item, defined, .. } if item == key => {
                Some((Item::from(defined), position + 1))
            }
            _ => None,
        });
    let since = defined.as_ref().map_or(0, |(_, position)| *position);
    let edited = approved_params(&history[since..], key, steps);
    (defined.map(|(item, _)| item), edited)
}

impl From<&DefinedItem> for Item {
    fn from(defined: &DefinedItem) -> Self {
        Self {
            id: defined.id.clone(),
            version: defined.version.clone(),
            title: defined.title.clone(),
            data: defined.data.clone(),
        }
    }
}
