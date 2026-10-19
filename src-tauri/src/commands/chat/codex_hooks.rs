use serde::Deserialize;
use serde_json::{json, Value};

use super::model::{HookControls, HookOption, HookSetting};

#[derive(Deserialize)]
struct HookError {
    path: String,
    message: String,
}

#[derive(Deserialize)]
struct HookEntry {
    hooks: Vec<HookOption>,
    errors: Vec<HookError>,
    warnings: Vec<String>,
}

pub fn parse(result: &Value, cwd: &str) -> Result<HookControls, String> {
    let entry = result["data"]
        .as_array()
        .and_then(|entries| entries.iter().find(|entry| entry["cwd"] == cwd))
        .ok_or("Codex did not return hooks for this worktree")?;
    let entry: HookEntry = serde_json::from_value(entry.clone())
        .map_err(|_| "Codex returned an unsupported hook definition; update codex-cli")?;
    let errors = entry
        .errors
        .iter()
        .map(|error| format!("{}: {}", error.path, error.message))
        .collect::<Vec<_>>();
    Ok(HookControls {
        items: entry.hooks,
        loading: false,
        error: (!errors.is_empty()).then(|| errors.join("\n")),
        warnings: entry.warnings,
    })
}

pub fn write_params(hooks: &HookControls, setting: &HookSetting) -> Result<Value, String> {
    if hooks.loading {
        return Err("Wait for the hook update to finish".into());
    }
    let hook = hooks
        .items
        .iter()
        .find(|hook| hook.key == setting.key && hook.current_hash == setting.hash)
        .ok_or("The hook definition changed; refresh and review it again")?;
    if hook.is_managed {
        return Err("Managed hooks cannot be changed".into());
    }
    if setting.trust && (hook.handler_type != "command" || hook.command.is_none()) {
        return Err("Review this hook in Codex CLI; its definition is not available here".into());
    }
    let key = serde_json::to_string(&hook.key).map_err(|_| "Invalid hook key")?;
    let mut edits = Vec::new();
    if setting.trust {
        edits.push(json!({
            "keyPath": format!("hooks.state.{key}.trusted_hash"),
            "value": hook.current_hash,
            "mergeStrategy": "replace"
        }));
    }
    if let Some(enabled) = setting.enabled {
        edits.push(json!({
            "keyPath": format!("hooks.state.{key}.enabled"),
            "value": enabled,
            "mergeStrategy": "replace"
        }));
    }
    if edits.is_empty() {
        return Err("No hook change was requested".into());
    }
    Ok(json!({ "edits": edits, "reloadUserConfig": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition() -> Value {
        json!({
            "key": "/tmp/a.b/\"hooks\".json:pre_tool_use:0:0",
            "currentHash": "sha256:reviewed",
            "eventName": "preToolUse",
            "sourcePath": "/tmp/a.b/hooks.json",
            "enabled": false,
            "isManaged": false,
            "trustStatus": "modified",
            "handlerType": "command",
            "command": "wtm --cli agent event waiting --agent codex"
        })
    }

    fn controls() -> HookControls {
        parse(
            &json!({ "data": [{
            "cwd": "/wt/a", "hooks": [definition()], "errors": [], "warnings": []
        }] }),
            "/wt/a",
        )
        .unwrap()
    }

    fn setting() -> HookSetting {
        HookSetting {
            key: definition()["key"].as_str().unwrap().into(),
            hash: "sha256:reviewed".into(),
            enabled: Some(true),
            trust: true,
        }
    }

    #[test]
    fn lists_the_current_worktree_and_preserves_load_errors() {
        let result = json!({ "data": [
            { "cwd": "/wt/other", "hooks": [], "errors": [], "warnings": [] },
            { "cwd": "/wt/a", "hooks": [definition()], "warnings": ["Duplicate sources"],
              "errors": [{ "path": "/bad/hooks.json", "message": "Invalid JSON" }] }
        ] });
        let hooks = parse(&result, "/wt/a").unwrap();
        assert_eq!(hooks.items.len(), 1);
        assert_eq!(hooks.items[0].trust_status, "modified");
        assert_eq!(
            hooks.error.as_deref(),
            Some("/bad/hooks.json: Invalid JSON")
        );
        assert_eq!(hooks.warnings, ["Duplicate sources"]);
        assert!(parse(&result, "/missing").is_err());
        assert!(parse(&json!({"data":[{"cwd":"/wt/a"}]}), "/wt/a").is_err());
    }

    #[test]
    fn trusts_only_the_reviewed_hash_and_quotes_the_full_hook_key() {
        let params = write_params(&controls(), &setting()).unwrap();
        let quoted_key = serde_json::to_string(&setting().key).unwrap();
        assert_eq!(
            params,
            json!({
                "edits": [
                    { "keyPath": format!("hooks.state.{quoted_key}.trusted_hash"), "value": "sha256:reviewed", "mergeStrategy": "replace" },
                    { "keyPath": format!("hooks.state.{quoted_key}.enabled"), "value": true, "mergeStrategy": "replace" }
                ],
                "reloadUserConfig": true
            })
        );
    }

    #[test]
    fn rejects_changed_unknown_managed_and_pending_hooks() {
        let mut hooks = controls();
        let mut change = setting();
        change.hash = "sha256:different".into();
        assert!(write_params(&hooks, &change).is_err());
        change = setting();
        change.key = "unknown".into();
        assert!(write_params(&hooks, &change).is_err());
        hooks.items[0].is_managed = true;
        assert!(write_params(&hooks, &setting()).is_err());
        hooks.items[0].is_managed = false;
        hooks.loading = true;
        assert!(write_params(&hooks, &setting()).is_err());
    }

    #[test]
    fn disabling_does_not_trust_and_hidden_definitions_cannot_be_trusted() {
        let mut hooks = controls();
        for handler in ["prompt", "agent", "mcpTool", "command"] {
            hooks.items[0].handler_type = handler.into();
            hooks.items[0].command = None;
            hooks.items[0].server = Some("server".into());
            hooks.items[0].tool = Some("tool".into());
            assert!(write_params(&hooks, &setting()).is_err(), "{handler}");
        }
        let mut change = setting();
        change.trust = false;
        change.enabled = Some(false);
        let params = write_params(&hooks, &change).unwrap();
        assert_eq!(params["edits"].as_array().unwrap().len(), 1);
        assert_eq!(params["edits"][0]["value"], false);
        change.enabled = None;
        assert!(write_params(&hooks, &change).is_err());
    }

    #[test]
    fn trusted_mcp_hooks_can_be_enabled_without_granting_new_trust() {
        let mut hooks = controls();
        hooks.items[0].handler_type = "mcpTool".into();
        hooks.items[0].command = None;
        hooks.items[0].trust_status = "trusted".into();
        let mut change = setting();
        change.trust = false;
        let params = write_params(&hooks, &change).unwrap();
        assert_eq!(params["edits"].as_array().unwrap().len(), 1);
        assert_eq!(params["edits"][0]["value"], true);
    }
}
