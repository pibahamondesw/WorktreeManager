//! The contract of `selector.agent`: the schema its final reply is held to, and how that reply
//! becomes the candidates and cursor of the selection pass.

use serde_json::{json, Map, Value};

use super::{check_candidate, Candidate, SelectorOutput};

#[cfg(test)]
mod tests;

/// Strict enough for every agent's structured output: all fields required, none extra, so an
/// item's free-form `data` travels as the text of a JSON object.
pub fn result_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["items", "cursor"],
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "version", "title", "data"],
                    "properties": {
                        "id": { "type": "string" },
                        "version": { "type": "string" },
                        "title": { "type": ["string", "null"] },
                        "data": { "type": "string" }
                    }
                }
            },
            "cursor": { "type": ["string", "null"] }
        }
    })
}

/// The pass's candidates and cursor, with the result as read, for the run log.
pub struct AgentSelection {
    pub output: SelectorOutput,
    pub result: Value,
}

/// Reads the agent's final reply. Agents enforce the schema; this checks every item against the
/// same contract as a script's.
pub fn read_result(reply: Option<&str>) -> Result<AgentSelection, String> {
    let reply = reply
        .map(str::trim)
        .filter(|reply| !reply.is_empty())
        .ok_or("the agent replied without a result")?;
    let result: Value =
        serde_json::from_str(reply).map_err(|e| format!("the reply is not JSON: {e}"))?;
    let items = result["items"]
        .as_array()
        .ok_or("the result has no items list")?
        .iter()
        .enumerate()
        .map(|(index, item)| candidate(item).map_err(|error| format!("items[{index}]: {error}")))
        .collect::<Result<Vec<_>, _>>()?;
    for item in &items {
        check_candidate(item)?;
    }
    let cursor = match &result["cursor"] {
        Value::Null => None,
        Value::String(cursor) => Some(Value::String(cursor.clone())),
        _ => return Err("cursor must be a string or null".into()),
    };
    Ok(AgentSelection {
        output: SelectorOutput::Items { items, cursor },
        result,
    })
}

fn candidate(item: &Value) -> Result<Candidate, String> {
    let text = |key: &str| {
        item[key]
            .as_str()
            .map(String::from)
            .ok_or_else(|| format!("{key} must be a string"))
    };
    let title = match &item["title"] {
        Value::Null => None,
        Value::String(title) => Some(title.clone()),
        _ => return Err("title must be a string or null".into()),
    };
    Ok(Candidate {
        id: text("id")?,
        version: text("version")?,
        title,
        data: data(&item["data"])?,
    })
}

/// The text of a JSON object; an empty text, or none, is no data.
fn data(value: &Value) -> Result<Map<String, Value>, String> {
    match value {
        Value::Null => Ok(Map::new()),
        Value::Object(data) => Ok(data.clone()),
        Value::String(text) if text.trim().is_empty() => Ok(Map::new()),
        Value::String(text) => match serde_json::from_str(text) {
            Ok(Value::Object(data)) => Ok(data),
            _ => Err("data must be the text of a JSON object".into()),
        },
        _ => Err("data must be the text of a JSON object".into()),
    }
}
