//! The structured result of an agent step: the schema its session is held to and how its final
//! reply is read back into the verdict that decides the transition.

use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::super::definition::ResultSchema;
use super::super::store::events::{StepOutcome, Verdict};

pub(super) struct StepResult {
    pub value: Value,
    pub verdict: Option<Verdict>,
}

impl StepResult {
    pub fn outcome(&self) -> StepOutcome {
        match self.verdict {
            Some(Verdict::Reject) => StepOutcome::Reject,
            Some(Verdict::Noop) => StepOutcome::Noop,
            _ => StepOutcome::Success,
        }
    }
}

/// Strict enough for every agent's structured output: all fields required, none extra.
pub(super) fn verdict_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["verdict", "summary", "findings"],
        "properties": {
            "verdict": { "type": "string", "enum": ["pass", "reject", "blocked", "defer", "noop"] },
            "summary": { "type": "string" },
            "findings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["title", "detail", "path", "line"],
                    "properties": {
                        "title": { "type": "string" },
                        "detail": { "type": "string" },
                        "path": { "type": ["string", "null"] },
                        "line": { "type": ["integer", "null"] }
                    }
                }
            }
        }
    })
}

/// The schema a step's session is held to; custom schemas live in the automation folder.
pub(super) fn schema_of(result: &ResultSchema, folder: &Path) -> Result<Value, String> {
    let ResultSchema::Custom(path) = result else {
        return Ok(verdict_schema());
    };
    let text = fs::read_to_string(folder.join(path)).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
}

/// Reads the final reply as the step's result. Agents enforce the schema; this only checks what
/// the engine relies on.
pub(super) fn read(reply: Option<&str>, schema: &ResultSchema) -> Result<StepResult, String> {
    let reply = reply
        .map(str::trim)
        .filter(|reply| !reply.is_empty())
        .ok_or("the agent replied without a result")?;
    let value: Value =
        serde_json::from_str(reply).map_err(|e| format!("the reply is not JSON: {e}"))?;
    let object = value.as_object().ok_or("the result is not a JSON object")?;
    let verdict = match object.get("verdict") {
        Some(verdict) => Some(
            serde_json::from_value(verdict.clone())
                .map_err(|_| "verdict must be pass, reject, blocked, defer or noop")?,
        ),
        None if *schema == ResultSchema::Verdict => return Err("the result has no verdict".into()),
        None => None,
    };
    if *schema == ResultSchema::Verdict {
        check_verdict_fields(&value)?;
    }
    Ok(StepResult { value, verdict })
}

fn check_verdict_fields(value: &Value) -> Result<(), String> {
    if !value["summary"].is_string() {
        return Err("summary must be a string".into());
    }
    let findings = value["findings"]
        .as_array()
        .ok_or("findings must be a list")?;
    let readable = |finding: &Value| finding["title"].is_string() && finding["detail"].is_string();
    if !findings.iter().all(readable) {
        return Err("every finding needs a title and a detail".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(reply: &str) -> Result<StepResult, String> {
        read(Some(reply), &ResultSchema::Verdict)
    }

    #[test]
    fn verdicts_decide_the_outcome() {
        let outcomes: Vec<_> = ["pass", "reject", "blocked", "defer", "noop"]
            .into_iter()
            .map(|name| {
                let reply = json!({ "verdict": name, "summary": "s", "findings": [] });
                verdict(&reply.to_string()).unwrap().outcome()
            })
            .collect();
        assert_eq!(
            outcomes,
            [
                StepOutcome::Success,
                StepOutcome::Reject,
                StepOutcome::Success,
                StepOutcome::Success,
                StepOutcome::Noop,
            ]
        );
        let finding = json!({ "title": "t", "detail": "d", "path": null, "line": null });
        let reply = json!({ "verdict": "reject", "summary": "s", "findings": [finding] });
        let result = verdict(&format!("\n{reply}\n")).unwrap();
        assert_eq!(result.verdict, Some(Verdict::Reject));
        assert_eq!(result.value, reply);
    }

    #[test]
    fn unreadable_results_say_why() {
        let errors: Vec<_> = [
            "",
            "not json",
            "[1]",
            r#"{"summary":"s","findings":[]}"#,
            r#"{"verdict":"maybe","summary":"s","findings":[]}"#,
            r#"{"verdict":"pass","findings":[]}"#,
            r#"{"verdict":"pass","summary":"s"}"#,
            r#"{"verdict":"pass","summary":"s","findings":[{"title":"t"}]}"#,
        ]
        .into_iter()
        .map(|reply| verdict(reply).err().unwrap())
        .collect();
        assert_eq!(errors[0], "the agent replied without a result");
        assert!(errors[1].starts_with("the reply is not JSON"));
        assert_eq!(
            errors[2..],
            [
                "the result is not a JSON object",
                "the result has no verdict",
                "verdict must be pass, reject, blocked, defer or noop",
                "summary must be a string",
                "findings must be a list",
                "every finding needs a title and a detail",
            ]
        );
        assert!(read(None, &ResultSchema::Verdict).is_err());
    }

    #[test]
    fn custom_results_only_need_a_valid_verdict_when_they_have_one() {
        let custom = ResultSchema::Custom("schemas/x.json".into());
        let plain = read(Some(r#"{"files":[]}"#), &custom).unwrap();
        assert_eq!(
            (plain.verdict, plain.outcome()),
            (None, StepOutcome::Success)
        );
        assert!(read(Some(r#"{"verdict":"nope"}"#), &custom).is_err());
        let blocked = read(Some(r#"{"verdict":"blocked"}"#), &custom).unwrap();
        assert_eq!(blocked.verdict, Some(Verdict::Blocked));
    }

    #[test]
    fn custom_schemas_are_read_from_the_automation_folder() {
        let folder = std::env::temp_dir().join(format!("wtm-schema-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(folder.join("schemas")).unwrap();
        fs::write(folder.join("schemas/ok.json"), r#"{"type":"object"}"#).unwrap();
        fs::write(folder.join("schemas/bad.json"), "{").unwrap();
        let custom = |name: &str| ResultSchema::Custom(format!("schemas/{name}.json"));
        assert_eq!(
            schema_of(&custom("ok"), &folder),
            Ok(json!({ "type": "object" }))
        );
        assert!(schema_of(&custom("bad"), &folder)
            .unwrap_err()
            .starts_with("schemas/bad.json: "));
        assert!(schema_of(&custom("gone"), &folder)
            .unwrap_err()
            .starts_with("schemas/gone.json: "));
        assert_eq!(
            schema_of(&ResultSchema::Verdict, &folder),
            Ok(verdict_schema())
        );
        fs::remove_dir_all(folder).unwrap();
    }
}
