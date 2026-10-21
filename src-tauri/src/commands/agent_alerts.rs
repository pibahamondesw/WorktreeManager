//! Agent activity alerts: the global Claude Code / Codex hooks that report "working", "waiting"
//! and "done" through `wtm agent event`, plus the sounds, system notifications and Dock badge
//! the app shows for them.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::agent_notifications;
use super::agents::Agents;
use super::user_file::UserFile;

const LOCAL_PROFILE: &str = "com.worktreemanager.dev.local";
const OWNED_COMMAND_MARKER: &str = " agent event ";
pub const NOTIFICATION_OPEN_EVENT: &str = "agent-notification-open";
/// Set on every embedded agent process so its hooks report the task and surface directly.
pub const TASK_ENV: &str = "WTM_TASK_ID";
pub const SURFACE_ENV: &str = "WTM_SURFACE";

pub(crate) struct HookSpec {
    pub event: &'static str,
    pub matcher: Option<&'static str>,
    pub state: &'static str,
}

/// Where and how one agent's alert hooks are installed.
pub(crate) struct AlertHooks {
    pub agent: &'static str,
    pub config: PathBuf,
    pub specs: &'static [HookSpec],
    /// Run the handlers asynchronously so they never delay the agent.
    pub async_handlers: bool,
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// The app a hook reports to: the executable it runs and the profile whose socket it reaches.
fn hook_target(executable: &str, local: bool) -> String {
    format!(
        "{} --cli{}",
        shell_quote(executable),
        if local { " --local" } else { "" },
    )
}

fn hook_command(target: &str, agent: &str, state: &str) -> String {
    format!("{target}{OWNED_COMMAND_MARKER}{state} --agent {agent}")
}

fn owned_handler_target(handler: &Value) -> Option<&str> {
    let command = handler["command"].as_str()?;
    let (target, _) = command.rsplit_once(OWNED_COMMAND_MARKER)?;
    target.contains(" --cli").then_some(target)
}

fn is_owned_handler(handler: &Value) -> bool {
    owned_handler_target(handler).is_some()
}

fn group_handlers(group: &Value) -> impl Iterator<Item = &Value> {
    group["hooks"].as_array().into_iter().flatten()
}

fn owned_handlers(root: &Value) -> impl Iterator<Item = &Value> {
    root["hooks"]
        .as_object()
        .into_iter()
        .flat_map(|events| events.values())
        .filter_map(Value::as_array)
        .flatten()
        .flat_map(group_handlers)
        .filter(|handler| is_owned_handler(handler))
}

fn has_owned_hooks(root: &Value) -> bool {
    owned_handlers(root).next().is_some()
}

/// Where WorktreeManager hooks installed by another app (another build or profile) report to.
fn other_app_target(root: &Value, target: &str) -> Option<String> {
    owned_handlers(root)
        .filter_map(owned_handler_target)
        .find(|owned| *owned != target)
        .map(String::from)
}

/// Drop every WorktreeManager handler, and the groups/events that only held them. Everything the
/// user configured is kept untouched.
fn remove_owned_hooks(root: &mut Value) {
    remove_handlers(root, is_owned_handler);
}

fn remove_app_hooks(root: &mut Value, target: &str) {
    remove_handlers(root, |handler| {
        owned_handler_target(handler) == Some(target)
    });
}

fn remove_handlers(root: &mut Value, matches: impl Fn(&Value) -> bool) {
    let Some(events) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    events.retain(|_, groups| {
        let Some(groups) = groups.as_array_mut() else {
            return true;
        };
        let before = groups.len();
        groups.retain_mut(|group| {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let owned_before = handlers.len();
            handlers.retain(|handler| !matches(handler));
            owned_before == handlers.len() || !handlers.is_empty()
        });
        before == groups.len() || !groups.is_empty()
    });
}

fn install_hooks(root: &mut Value, hooks: &AlertHooks, target: &str) -> Result<(), String> {
    let Some(object) = root.as_object_mut() else {
        return Err("The agent config is not a JSON object".into());
    };
    let events = object
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    if !events.is_object() {
        return Err("The agent config has an unexpected \"hooks\" value".into());
    }
    remove_owned_hooks(root);
    let events = root["hooks"].as_object_mut().expect("hooks is an object");
    for spec in hooks.specs {
        let command = hook_command(target, hooks.agent, spec.state);
        let mut handler = json!({ "type": "command", "command": command, "timeout": 5 });
        if hooks.async_handlers {
            handler["async"] = json!(true);
        }
        let mut group = json!({ "hooks": [handler] });
        if let Some(matcher) = spec.matcher {
            group["matcher"] = json!(matcher);
        }
        let groups = events
            .entry(spec.event)
            .or_insert_with(|| Value::Array(Vec::new()));
        match groups.as_array_mut() {
            Some(groups) => groups.push(group),
            None => {
                return Err(format!(
                    "The agent config has an unexpected \"{}\" hook",
                    spec.event
                ))
            }
        }
    }
    Ok(())
}

fn load_config(path: &Path) -> Result<Option<(String, Value)>, String> {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .map(|root| Some((content, root)))
            .map_err(|_| {
                format!(
                    "{} is not valid JSON; fix it before changing hooks",
                    path.display()
                )
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Cannot read {}: {error}", path.display())),
    }
}

fn write_config(path: &Path, original: &str, root: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    }
    let out =
        serde_json::to_string_pretty(root).map_err(|e| format!("Cannot serialize config: {e}"))?;
    UserFile::new(path)
        .unchanged_from(original)
        .replace(out + "\n")
}

#[derive(Serialize, PartialEq, Debug)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum HooksState {
    Missing,
    /// Installed by an older version of this app; reinstalling brings them up to date.
    Outdated,
    Installed,
    /// Another WorktreeManager (another build or profile) receives the events; replacing its
    /// hooks needs confirmation, since it would stop alerting.
    OtherApp {
        target: String,
    },
}

#[derive(Serialize)]
pub struct AgentHooksStatus {
    claude: HooksState,
    codex: HooksState,
}

fn hooks_state(root: &Value, hooks: &AlertHooks, target: &str) -> HooksState {
    if let Some(target) = other_app_target(root, target) {
        return HooksState::OtherApp { target };
    }
    if !has_owned_hooks(root) {
        return HooksState::Missing;
    }
    let mut expected = root.clone();
    match install_hooks(&mut expected, hooks, target) {
        Ok(()) if expected == *root => HooksState::Installed,
        _ => HooksState::Outdated,
    }
}

fn app_hook_target<R: Runtime>(app: &AppHandle<R>) -> Result<String, String> {
    let executable = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|_| "Cannot locate the WorktreeManager executable")?;
    let local = app.config().identifier == LOCAL_PROFILE;
    Ok(hook_target(&executable.to_string_lossy(), local))
}

fn ensure_no_other_app(root: &Value, target: &str, replace_other_app: bool) -> Result<(), String> {
    match other_app_target(root, target) {
        Some(other) if !replace_other_app => Err(format!(
            "These hooks report to another WorktreeManager ({other}); confirm to replace them"
        )),
        _ => Ok(()),
    }
}

fn installed_state<R: Runtime>(app: &AppHandle<R>, agents: &Agents, agent: &str) -> HooksState {
    let Ok(hooks) = agents.get(agent).map(|adapter| adapter.alert_hooks()) else {
        return HooksState::Missing;
    };
    let Ok(Some((_, root))) = load_config(&hooks.config) else {
        return HooksState::Missing;
    };
    match app_hook_target(app) {
        Ok(target) => hooks_state(&root, &hooks, &target),
        Err(_) => HooksState::Outdated,
    }
}

#[tauri::command]
pub fn agent_hooks_status<R: Runtime>(app: AppHandle<R>) -> AgentHooksStatus {
    let agents = Agents::of(&app);
    AgentHooksStatus {
        claude: installed_state(&app, &agents, "claude"),
        codex: installed_state(&app, &agents, "codex"),
    }
}

#[tauri::command]
pub fn agent_hooks_install<R: Runtime>(
    app: AppHandle<R>,
    agent: String,
    replace_other_app: bool,
) -> Result<(), String> {
    let hooks = Agents::of(&app).get(&agent)?.alert_hooks();
    let (original, mut root) =
        load_config(&hooks.config)?.unwrap_or_else(|| (String::new(), json!({})));
    let target = app_hook_target(&app)?;
    ensure_no_other_app(&root, &target, replace_other_app)?;
    install_hooks(&mut root, &hooks, &target)?;
    write_config(&hooks.config, &original, &root)
}

#[tauri::command]
pub fn agent_hooks_remove<R: Runtime>(app: AppHandle<R>, agent: String) -> Result<(), String> {
    let path = Agents::of(&app).get(&agent)?.alert_hooks().config;
    let Some((original, mut root)) = load_config(&path)? else {
        return Ok(());
    };
    let target = app_hook_target(&app)?;
    let before = root.clone();
    remove_app_hooks(&mut root, &target);
    if root == before {
        return Ok(());
    }
    write_config(&path, &original, &root)
}

const SYSTEM_SOUNDS: &[&str] = &[
    "Basso",
    "Blow",
    "Bottle",
    "Frog",
    "Funk",
    "Glass",
    "Hero",
    "Morse",
    "Ping",
    "Pop",
    "Purr",
    "Sosumi",
    "Submarine",
    "Tink",
];

pub fn play_sound(sound: &str) {
    if !SYSTEM_SOUNDS.contains(&sound) {
        return;
    }
    let _ = Command::new("/usr/bin/afplay")
        .arg(format!("/System/Library/Sounds/{sound}.aiff"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Where a notification click leads: the task, and the surface the agent runs in.
#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NotificationTarget {
    pub(crate) task_id: String,
    surface: String,
    agent: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentNotification {
    title: String,
    body: String,
    target: NotificationTarget,
    #[serde(default)]
    task_open: bool,
}

/// `get_window`, not `get_webview_window`: once embedded VS Code adds a child webview, the main
/// window hosts several webviews and is no longer returned as a webview window.
fn app_in_foreground(app: &AppHandle) -> bool {
    app.get_window("main")
        .and_then(|window| window.is_focused().ok())
        .unwrap_or(false)
}

pub(crate) fn reveal_target(app: &AppHandle, target: &NotificationTarget) {
    if let Some(window) = app.get_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = app.emit_to("main", NOTIFICATION_OPEN_EVENT, target.clone());
}

fn hidden_by_open_task(task_open: bool, app_in_foreground: bool) -> bool {
    task_open && app_in_foreground
}

/// Sound for a finished or waiting agent, plus a system notification that opens the task when
/// clicked. The notification is skipped while its task is open in the focused app.
#[tauri::command]
pub fn agent_attention(
    app: AppHandle,
    sound: Option<String>,
    notification: Option<AgentNotification>,
) {
    if let Some(sound) = sound {
        play_sound(&sound);
    }
    if let Some(notification) =
        notification.filter(|n| !hidden_by_open_task(n.task_open, app_in_foreground(&app)))
    {
        agent_notifications::deliver(
            &app,
            notification.title,
            notification.body,
            notification.target,
        );
    }
}

#[tauri::command]
pub fn agent_dismiss_notifications(app: AppHandle, task_ids: Vec<String>) {
    agent_notifications::dismiss(&app, task_ids);
}

#[tauri::command]
pub fn agent_badge(app: AppHandle, count: u32) {
    if let Some(window) = app.get_window("main") {
        let _ = window.set_badge_count((count > 0).then_some(i64::from(count)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hooks(agent: &str) -> AlertHooks {
        Agents::default().get(agent).unwrap().alert_hooks()
    }

    const TARGET: &str = r"'/Apps/WTM'\''s.app/wtm' --cli";
    const DEV_TARGET: &str = "'/dev/target/debug/wtm' --cli --local";

    fn command(state: &str) -> String {
        hook_command(TARGET, "claude", state)
    }

    #[test]
    fn notifications_are_hidden_only_for_the_open_task_in_the_focused_app() {
        assert!(hidden_by_open_task(true, true));
        assert!(!hidden_by_open_task(false, true));
        assert!(!hidden_by_open_task(true, false));
    }

    #[test]
    fn hook_command_quotes_the_executable_and_marks_the_profile() {
        assert_eq!(
            command("done"),
            r"'/Apps/WTM'\''s.app/wtm' --cli agent event done --agent claude"
        );
        assert_eq!(hook_target("/Apps/WTM's.app/wtm", false), TARGET);
        let odd_path = hook_target("/x agent event y/wtm", false);
        let odd = json!({ "command": hook_command(&odd_path, "claude", "done") });
        assert_eq!(owned_handler_target(&odd), Some(odd_path.as_str()));
        assert_eq!(
            hook_command(&hook_target("/b/wtm", true), "codex", "waiting"),
            "'/b/wtm' --cli --local agent event waiting --agent codex"
        );
    }

    #[test]
    fn install_keeps_user_hooks_and_settings() {
        let mut root = json!({
            "model": "opus",
            "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "afplay x" }] }],
                "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "rtk" }] }]
            }
        });
        install_hooks(&mut root, &hooks("claude"), TARGET).unwrap();

        assert_eq!(root["model"], "opus");
        assert_eq!(root["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "rtk");
        let stop = root["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "afplay x");
        assert_eq!(stop[1]["hooks"][0]["command"], command("done"));
        assert_eq!(stop[1]["hooks"][0]["async"], true);
        assert_eq!(
            root["hooks"]["Notification"][0]["matcher"],
            "permission_prompt|elicitation_dialog"
        );
        assert!(has_owned_hooks(&root));
        assert_eq!(
            root["hooks"]["PreToolUse"][1]["matcher"],
            "^AskUserQuestion$"
        );
    }

    #[test]
    fn state_tells_missing_current_and_outdated_hooks_apart() {
        let mut root = json!({ "hooks": { "Stop": [] } });
        assert_eq!(
            hooks_state(&root, &hooks("claude"), TARGET),
            HooksState::Missing
        );
        install_hooks(&mut root, &hooks("claude"), TARGET).unwrap();
        assert_eq!(
            hooks_state(&root, &hooks("claude"), TARGET),
            HooksState::Installed
        );
        root["hooks"]["PreToolUse"].as_array_mut().unwrap().clear();
        assert_eq!(
            hooks_state(&root, &hooks("claude"), TARGET),
            HooksState::Outdated
        );
    }

    #[test]
    fn hooks_of_another_app_are_reported_and_only_replaced_on_confirmation() {
        let mut root = json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "afplay x" }] }] } });
        install_hooks(&mut root, &hooks("claude"), DEV_TARGET).unwrap();
        let other_app = HooksState::OtherApp {
            target: DEV_TARGET.into(),
        };
        assert_eq!(hooks_state(&root, &hooks("claude"), TARGET), other_app);
        assert_eq!(
            hooks_state(&root, &hooks("claude"), DEV_TARGET),
            HooksState::Installed
        );
        assert_eq!(
            serde_json::to_value(&other_app).unwrap(),
            json!({ "state": "otherApp", "target": DEV_TARGET })
        );
        assert!(ensure_no_other_app(&root, TARGET, false)
            .unwrap_err()
            .contains(DEV_TARGET));
        assert!(ensure_no_other_app(&root, DEV_TARGET, false).is_ok());
        assert!(ensure_no_other_app(&root, TARGET, true).is_ok());

        let before = root.clone();
        remove_app_hooks(&mut root, TARGET);
        assert_eq!(root, before);

        install_hooks(&mut root, &hooks("claude"), TARGET).unwrap();
        assert_eq!(
            hooks_state(&root, &hooks("claude"), TARGET),
            HooksState::Installed
        );
        assert!(!root.to_string().contains("/dev/target"));
        assert_eq!(root["hooks"]["Stop"][0]["hooks"][0]["command"], "afplay x");
    }

    #[test]
    fn reinstall_replaces_previous_owned_hooks() {
        let mut root = json!({});
        install_hooks(&mut root, &hooks("codex"), "'/old/wtm' --cli").unwrap();
        install_hooks(&mut root, &hooks("codex"), "'/new/wtm' --cli").unwrap();

        let serialized = root.to_string();
        assert!(!serialized.contains("/old/wtm"));
        assert_eq!(root["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(
            root["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"],
            5
        );
        assert!(root["hooks"]["Stop"][0]["hooks"][0].get("async").is_none());
    }

    #[test]
    fn remove_drops_only_owned_handlers_and_emptied_entries() {
        let mut root = json!({
            "hooks": {
                "Stop": [{ "hooks": [
                    { "type": "command", "command": "afplay x" },
                    { "type": "command", "command": command("done") }
                ] }],
                "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": command("working") }] }],
                "SessionStart": []
            }
        });
        remove_owned_hooks(&mut root);

        assert_eq!(
            root["hooks"]["Stop"],
            json!([{ "hooks": [{ "type": "command", "command": "afplay x" }] }])
        );
        assert!(root["hooks"].get("UserPromptSubmit").is_none());
        assert_eq!(root["hooks"]["SessionStart"], json!([]));
        assert!(!has_owned_hooks(&root));
    }

    #[test]
    fn load_config_rejects_invalid_json() {
        let path = std::env::temp_dir().join(format!("wm-hooks-{}.json", uuid::Uuid::new_v4()));
        fs::write(&path, "{invalid").unwrap();

        let error = load_config(&path).unwrap_err();
        fs::remove_file(&path).unwrap();

        assert!(error.ends_with("is not valid JSON; fix it before changing hooks"));
    }

    #[test]
    fn install_rejects_unexpected_shapes() {
        assert!(install_hooks(&mut json!([]), &hooks("claude"), TARGET).is_err());
        assert!(install_hooks(&mut json!({ "hooks": [] }), &hooks("claude"), TARGET).is_err());
        assert!(install_hooks(
            &mut json!({ "hooks": { "Stop": {} } }),
            &hooks("claude"),
            TARGET
        )
        .is_err());
    }

    #[test]
    fn commands_install_report_and_remove_hooks_in_each_agent_config() {
        use super::super::agents::{claude::Claude, codex::Codex};
        use tauri::test::{mock_builder, mock_context, noop_assets};
        let home = std::env::temp_dir().join(format!("wtm-alerts-{}", uuid::Uuid::new_v4()));
        let agents = Agents::new(
            Claude::new("claude", Some(home.clone())),
            Codex::new("codex", Some(home.clone()), None),
        );
        let app = mock_builder()
            .manage(agents)
            .build(mock_context(noop_assets()))
            .unwrap();
        let status = || {
            let status = agent_hooks_status(app.handle().clone());
            (status.claude, status.codex)
        };
        assert_eq!(status(), (HooksState::Missing, HooksState::Missing));
        assert_eq!(
            installed_state(app.handle(), &Agents::of(app.handle()), "gemini"),
            HooksState::Missing
        );
        agent_hooks_install(app.handle().clone(), "codex".into(), false).unwrap();
        assert_eq!(status(), (HooksState::Missing, HooksState::Installed));
        assert!(fs::read_to_string(home.join(".codex/hooks.json"))
            .unwrap()
            .contains("--agent codex"));
        assert!(agent_hooks_install(app.handle().clone(), "gemini".into(), false).is_err());
        assert!(agent_hooks_remove(app.handle().clone(), "gemini".into()).is_err());

        let claude_config = home.join(".claude/settings.json");
        let mut other_app = json!({});
        install_hooks(&mut other_app, &hooks("claude"), DEV_TARGET).unwrap();
        write_config(&claude_config, "", &other_app).unwrap();
        let reported_elsewhere = HooksState::OtherApp {
            target: DEV_TARGET.into(),
        };
        assert_eq!(status().0, reported_elsewhere);
        agent_hooks_remove(app.handle().clone(), "claude".into()).unwrap();
        assert_eq!(status().0, reported_elsewhere);
        assert!(agent_hooks_install(app.handle().clone(), "claude".into(), false).is_err());
        assert_eq!(status().0, reported_elsewhere);
        agent_hooks_install(app.handle().clone(), "claude".into(), true).unwrap();
        assert_eq!(status().0, HooksState::Installed);

        for agent in ["claude", "codex", "codex"] {
            agent_hooks_remove(app.handle().clone(), agent.into()).unwrap();
        }
        assert_eq!(status(), (HooksState::Missing, HooksState::Missing));
        fs::remove_dir_all(home).unwrap();
    }
}
