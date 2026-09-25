//! Agent activity alerts: the global Claude Code / Codex hooks that report "working", "waiting"
//! and "done" through `wtm agent event`, plus the sounds, system notifications and Dock badge
//! the app shows for them.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, Manager};

const LOCAL_PROFILE: &str = "com.worktreemanager.dev.local";
const OWNED_COMMAND_MARKER: &str = " agent event ";
pub const NOTIFICATION_OPEN_EVENT: &str = "agent-notification-open";
/// Set on every embedded agent process so its hooks report the task and surface directly.
pub const TASK_ENV: &str = "WTM_TASK_ID";
pub const SURFACE_ENV: &str = "WTM_SURFACE";

struct HookSpec {
    event: &'static str,
    matcher: Option<&'static str>,
    state: &'static str,
}

const CLAUDE_HOOKS: &[HookSpec] = &[
    HookSpec {
        event: "UserPromptSubmit",
        matcher: None,
        state: "working",
    },
    HookSpec {
        event: "PostToolUse",
        matcher: None,
        state: "working",
    },
    HookSpec {
        event: "PreToolUse",
        matcher: Some("^AskUserQuestion$"),
        state: "waiting",
    },
    HookSpec {
        event: "Notification",
        matcher: Some("permission_prompt|elicitation_dialog"),
        state: "waiting",
    },
    HookSpec {
        event: "Stop",
        matcher: None,
        state: "done",
    },
];

const CODEX_HOOKS: &[HookSpec] = &[
    HookSpec {
        event: "UserPromptSubmit",
        matcher: None,
        state: "working",
    },
    HookSpec {
        event: "PostToolUse",
        matcher: None,
        state: "working",
    },
    HookSpec {
        event: "PreToolUse",
        matcher: Some("request_user_input"),
        state: "waiting",
    },
    HookSpec {
        event: "PermissionRequest",
        matcher: None,
        state: "waiting",
    },
    HookSpec {
        event: "Stop",
        matcher: None,
        state: "done",
    },
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum HookAgent {
    Claude,
    Codex,
}

impl HookAgent {
    fn parse(agent: &str) -> Result<Self, String> {
        match agent {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            _ => Err("Unknown agent".into()),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn specs(self) -> &'static [HookSpec] {
        match self {
            Self::Claude => CLAUDE_HOOKS,
            Self::Codex => CODEX_HOOKS,
        }
    }

    fn config_path(self) -> PathBuf {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()));
        match self {
            Self::Claude => home.join(".claude").join("settings.json"),
            Self::Codex => home.join(".codex").join("hooks.json"),
        }
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn hook_command(executable: &str, local: bool, agent: HookAgent, state: &str) -> String {
    format!(
        "{} --cli{} agent event {state} --agent {}",
        shell_quote(executable),
        if local { " --local" } else { "" },
        agent.name()
    )
}

fn is_owned_handler(handler: &Value) -> bool {
    handler["command"]
        .as_str()
        .is_some_and(|command| command.contains(" --cli") && command.contains(OWNED_COMMAND_MARKER))
}

fn group_handlers(group: &Value) -> impl Iterator<Item = &Value> {
    group["hooks"].as_array().into_iter().flatten()
}

fn has_owned_hooks(root: &Value) -> bool {
    root["hooks"].as_object().is_some_and(|events| {
        events
            .values()
            .filter_map(Value::as_array)
            .flatten()
            .any(|group| group_handlers(group).any(is_owned_handler))
    })
}

/// Drop every handler this app owns, and the groups/events that only held them. Everything the
/// user configured is kept untouched.
fn remove_owned_hooks(root: &mut Value) {
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
            handlers.retain(|handler| !is_owned_handler(handler));
            owned_before == handlers.len() || !handlers.is_empty()
        });
        before == groups.len() || !groups.is_empty()
    });
}

fn install_hooks(
    root: &mut Value,
    agent: HookAgent,
    command: impl Fn(&str) -> String,
) -> Result<(), String> {
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
    for spec in agent.specs() {
        let mut handler =
            json!({ "type": "command", "command": command(spec.state), "timeout": 5 });
        if agent == HookAgent::Claude {
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

fn load_config(path: &Path) -> Result<Option<Value>, String> {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).map(Some).map_err(|_| {
            format!(
                "{} is not valid JSON; fix it before changing hooks",
                path.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Cannot read {}: {error}", path.display())),
    }
}

fn write_config(path: &Path, root: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    }
    let out =
        serde_json::to_string_pretty(root).map_err(|e| format!("Cannot serialize config: {e}"))?;
    let tmp = {
        let mut p = path.to_path_buf().into_os_string();
        p.push(".wm.tmp");
        PathBuf::from(p)
    };
    fs::write(&tmp, out + "\n").map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("Cannot finalize {}: {e}", path.display()))
}

#[derive(Serialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum HooksState {
    Missing,
    /// Installed by another build or an older version; reinstalling brings them up to date.
    Outdated,
    Installed,
}

#[derive(Serialize)]
pub struct AgentHooksStatus {
    claude: HooksState,
    codex: HooksState,
}

fn hooks_state(root: &Value, agent: HookAgent, command: impl Fn(&str) -> String) -> HooksState {
    if !has_owned_hooks(root) {
        return HooksState::Missing;
    }
    let mut expected = root.clone();
    match install_hooks(&mut expected, agent, command) {
        Ok(()) if expected == *root => HooksState::Installed,
        _ => HooksState::Outdated,
    }
}

fn app_hook_command(app: &AppHandle, agent: HookAgent) -> Result<impl Fn(&str) -> String, String> {
    let executable = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|_| "Cannot locate the WorktreeManager executable")?
        .to_string_lossy()
        .into_owned();
    let local = app.config().identifier == LOCAL_PROFILE;
    Ok(move |state: &str| hook_command(&executable, local, agent, state))
}

fn installed_state(app: &AppHandle, agent: HookAgent) -> HooksState {
    let Ok(Some(root)) = load_config(&agent.config_path()) else {
        return HooksState::Missing;
    };
    match app_hook_command(app, agent) {
        Ok(command) => hooks_state(&root, agent, command),
        Err(_) => HooksState::Outdated,
    }
}

#[tauri::command]
pub fn agent_hooks_status(app: AppHandle) -> AgentHooksStatus {
    AgentHooksStatus {
        claude: installed_state(&app, HookAgent::Claude),
        codex: installed_state(&app, HookAgent::Codex),
    }
}

#[tauri::command]
pub fn agent_hooks_install(app: AppHandle, agent: String) -> Result<(), String> {
    let agent = HookAgent::parse(&agent)?;
    let path = agent.config_path();
    let mut root = load_config(&path)?.unwrap_or_else(|| json!({}));
    install_hooks(&mut root, agent, app_hook_command(&app, agent)?)?;
    write_config(&path, &root)
}

#[tauri::command]
pub fn agent_hooks_remove(agent: String) -> Result<(), String> {
    let path = HookAgent::parse(&agent)?.config_path();
    let Some(mut root) = load_config(&path)? else {
        return Ok(());
    };
    if !has_owned_hooks(&root) {
        return Ok(());
    }
    remove_owned_hooks(&mut root);
    write_config(&path, &root)
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

pub fn default_sound(state: &str) -> Option<&'static str> {
    match state {
        "done" => Some("Submarine"),
        "waiting" => Some("Ping"),
        _ => None,
    }
}

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
    task_id: String,
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

fn reveal_target(app: &AppHandle, target: &NotificationTarget) {
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

/// `mac-notification-sys` uses `NSUserNotificationCenter`, which hides banners while the app is
/// active unless the delegate implements `userNotificationCenter:shouldPresentNotification:`.
/// Its delegate class lacks that method, so it is added once at runtime.
#[cfg(target_os = "macos")]
fn present_banners_while_active() {
    use objc2::encode::{Encode, Encoding};
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::{ffi, sel};

    extern "C-unwind" fn should_present(
        _delegate: *mut AnyObject,
        _cmd: Sel,
        _center: *mut AnyObject,
        _notification: *mut AnyObject,
    ) -> Bool {
        Bool::YES
    }

    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Some(class) = AnyClass::get(c"NotificationCenterDelegate") else {
            return;
        };
        let Ok(types) = std::ffi::CString::new(format!(
            "{}{}{}{}{}",
            Bool::ENCODING,
            Encoding::Object,
            Encoding::Sel,
            Encoding::Object,
            Encoding::Object
        )) else {
            return;
        };
        unsafe {
            let imp: Imp = std::mem::transmute(
                should_present
                    as extern "C-unwind" fn(
                        *mut AnyObject,
                        Sel,
                        *mut AnyObject,
                        *mut AnyObject,
                    ) -> Bool,
            );
            ffi::class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                sel!(userNotificationCenter:shouldPresentNotification:),
                imp,
                types.as_ptr(),
            );
        }
    });
}

#[cfg(target_os = "macos")]
fn send_notification(app: AppHandle, notification: AgentNotification) {
    std::thread::spawn(move || {
        let _ = mac_notification_sys::set_application(&app.config().identifier);
        present_banners_while_active();
        let response = mac_notification_sys::Notification::new()
            .title(&notification.title)
            .message(&notification.body)
            .wait_for_click(true)
            .send();
        if matches!(
            response,
            Ok(mac_notification_sys::NotificationResponse::Click)
        ) {
            reveal_target(&app, &notification.target);
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn send_notification(_app: AppHandle, _notification: AgentNotification) {}

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
        send_notification(app, notification);
    }
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

    fn command(state: &str) -> String {
        hook_command("/Apps/WTM's.app/wtm", false, HookAgent::Claude, state)
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
        assert_eq!(
            hook_command("/b/wtm", true, HookAgent::Codex, "waiting"),
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
        install_hooks(&mut root, HookAgent::Claude, command).unwrap();

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
            hooks_state(&root, HookAgent::Claude, command),
            HooksState::Missing
        );
        install_hooks(&mut root, HookAgent::Claude, command).unwrap();
        assert_eq!(
            hooks_state(&root, HookAgent::Claude, command),
            HooksState::Installed
        );
        let other_build = |s: &str| hook_command("/other/wtm", false, HookAgent::Claude, s);
        assert_eq!(
            hooks_state(&root, HookAgent::Claude, other_build),
            HooksState::Outdated
        );
        root["hooks"]["PreToolUse"].as_array_mut().unwrap().clear();
        assert_eq!(
            hooks_state(&root, HookAgent::Claude, command),
            HooksState::Outdated
        );
    }

    #[test]
    fn reinstall_replaces_previous_owned_hooks() {
        let mut root = json!({});
        install_hooks(&mut root, HookAgent::Codex, |s| {
            hook_command("/old/wtm", false, HookAgent::Codex, s)
        })
        .unwrap();
        install_hooks(&mut root, HookAgent::Codex, |s| {
            hook_command("/new/wtm", false, HookAgent::Codex, s)
        })
        .unwrap();

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
    fn install_rejects_unexpected_shapes() {
        assert!(install_hooks(&mut json!([]), HookAgent::Claude, command).is_err());
        assert!(install_hooks(&mut json!({ "hooks": [] }), HookAgent::Claude, command).is_err());
        assert!(install_hooks(
            &mut json!({ "hooks": { "Stop": {} } }),
            HookAgent::Claude,
            command
        )
        .is_err());
    }

    #[test]
    fn only_finished_and_waiting_have_default_sounds() {
        assert_eq!(default_sound("done"), Some("Submarine"));
        assert_eq!(default_sound("waiting"), Some("Ping"));
        assert_eq!(default_sound("working"), None);
        assert!(default_sound("done").is_some_and(|sound| SYSTEM_SOUNDS.contains(&sound)));
    }
}
