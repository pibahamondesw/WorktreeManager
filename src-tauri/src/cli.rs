use crate::automation::{failure, read_message, socket_path, Request, MAX_MESSAGE, VERSION};
use crate::commands::agent_alerts;
use serde_json::{json, Map, Value};
use std::{
    fs,
    io::{self, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

const HELP: &str = r#"wtm — control the running WorktreeManager app (JSON output)

Usage: wtm                         Open the app
       wtm [--local] <workspace|task> <operation> [id] [options]
       wtm [--local] agent event <working|waiting|done> --agent <claude|codex>
       worktree-manager --cli [--local] ...

workspace list
workspace get <id>
workspace create --input <file|->
workspace update <id> --input <file|->
workspace delete <id> [--keep-worktrees|--delete-worktrees] [--force]
task list [--workspace <id>]
task get <id> [--git]
task create --input <file|->
task link-issue <id> --issue <identifier>
task delete <id> <--keep-worktrees|--delete-worktrees> [--force]
agent event <state> --agent <name>   Agent hook: reads the hook JSON from stdin,
                                     prints nothing and always exits 0.

Workspace input: {"name":"Payments","repos":[{"localPath":"/repos/api"}]}
Repo fields: id (existing members only), name, localPath, worktreeBasePath.
Workspace update accepts name, repos (replacement list), linearOrgUrlKey (or null).
Task input: {"workspaceId":"...","branchName":"feature", "repoIds":["..."]}
Optional linearIssue: {"id":"uuid","identifier":"WOR-80","title":"Local CLI"}.
Use Linear's exact branchName alongside linearIssue. Without it, branch names
are namespaced under the Git username and get a suffix on collision.
Omit repoIds to include all repositories. Creation waits for setup and does not
open editors or modify Linear. Warnings indicate setup steps needing attention.

Examples:
  wtm
  wtm workspace list
  wtm task create --input task.json
  wtm task link-issue TASK_ID --issue WOR-123
  wtm task get TASK_ID --git
  wtm task delete TASK_ID --delete-worktrees

--local targets isolated development; default targets the installed/shared app.
--force only works with --delete-worktrees and discards uncommitted files.
Deletion keeps Git branches and archives notes. Workspace deletion needs a mode
when tasks exist. JSON goes to stdout; progress goes to stderr. Exit 0 = success,
1 = operation/connection failure, 2 = invalid arguments. Never retry mutations
automatically after a disconnect: an accepted operation can still finish.
--help and --version work without the app.
"#;

struct Arguments {
    local: bool,
    method: String,
    params: Map<String, Value>,
    input: Option<String>,
}

fn parse(args: &[String]) -> Result<Arguments, String> {
    let mut words = Vec::new();
    let mut params = Map::new();
    let mut local = false;
    let mut input = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--local" if !local => local = true,
            "--input" if input.is_none() => {
                input = Some(args.next().ok_or("--input requires a file or -")?.clone())
            }
            "--workspace" if !params.contains_key("workspaceId") => {
                params.insert(
                    "workspaceId".into(),
                    json!(args.next().ok_or("--workspace requires an ID")?),
                );
            }
            "--issue" if !params.contains_key("issue") => {
                params.insert(
                    "issue".into(),
                    json!(args.next().ok_or("--issue requires an identifier")?),
                );
            }
            "--agent" if !params.contains_key("agent") => {
                params.insert(
                    "agent".into(),
                    json!(args.next().ok_or("--agent requires claude or codex")?),
                );
            }
            "--git" if !params.contains_key("git") => {
                params.insert("git".into(), json!(true));
            }
            "--force" if !params.contains_key("force") => {
                params.insert("force".into(), json!(true));
            }
            "--keep-worktrees" | "--delete-worktrees"
                if !params.contains_key("deleteWorktrees") =>
            {
                params.insert("deleteWorktrees".into(), json!(arg == "--delete-worktrees"));
            }
            flag if flag.starts_with('-') => {
                return Err("Unknown, duplicated or conflicting option; see --help".into())
            }
            _ => words.push(arg.as_str()),
        }
    }
    if words.len() < 2 {
        return Err("Provide a resource and operation; see --help".into());
    }
    let method = format!("{}.{}", words[0], words[1]);
    let (needs_id, needs_input, allowed): (bool, bool, &[&str]) = match method.as_str() {
        "workspace.list" => (false, false, &[]),
        "workspace.get" => (true, false, &[]),
        "workspace.create" | "task.create" => (false, true, &[]),
        "workspace.update" => (true, true, &[]),
        "workspace.delete" | "task.delete" => (true, false, &["deleteWorktrees", "force"]),
        "task.list" => (false, false, &["workspaceId"]),
        "task.link-issue" => (true, false, &["issue"]),
        "task.get" => (true, false, &["git"]),
        "agent.event" => (true, false, &["agent"]),
        _ => return Err("Unknown operation; see --help".into()),
    };
    if words.len() != if needs_id { 3 } else { 2 }
        || needs_input != input.is_some()
        || params.keys().any(|key| !allowed.contains(&key.as_str()))
    {
        return Err("Invalid arguments for this operation; see --help".into());
    }
    if needs_id {
        params.insert("id".into(), json!(words[2]));
    }
    if method == "agent.event" {
        let state = params.remove("id").unwrap_or_default();
        if !["working", "waiting", "done"].contains(&state.as_str().unwrap_or_default())
            || !["claude", "codex"].contains(
                &params
                    .get("agent")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
        {
            return Err("Provide working, waiting or done and --agent claude or codex".into());
        }
        params.insert("state".into(), state);
    }
    if method == "task.link-issue" && !params.contains_key("issue") {
        return Err("Provide --issue with a Linear issue identifier".into());
    }
    if method == "task.delete" && !params.contains_key("deleteWorktrees") {
        return Err("Choose --keep-worktrees or --delete-worktrees".into());
    }
    if params.contains_key("force") && params.get("deleteWorktrees") != Some(&json!(true)) {
        return Err("--force requires --delete-worktrees".into());
    }
    Ok(Arguments {
        local,
        method,
        params,
        input,
    })
}

fn execute(mut arguments: Arguments) -> Result<Value, String> {
    if let Some(path) = arguments.input {
        let reader: Box<dyn Read> = if path == "-" {
            Box::new(io::stdin())
        } else {
            Box::new(fs::File::open(path).map_err(|_| "Cannot read input file")?)
        };
        let mut bytes = Vec::new();
        reader
            .take(MAX_MESSAGE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read input")?;
        if bytes.len() as u64 > MAX_MESSAGE {
            return Err("Input exceeds 1 MiB".into());
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| "Input must be valid JSON")?;
        if !value.is_object() {
            return Err("Input must be a JSON object".into());
        }
        arguments.params.insert("input".into(), value);
    }
    let profile = if arguments.local {
        "com.worktreemanager.dev.local"
    } else {
        "com.worktreemanager.dev"
    };
    let mut stream = UnixStream::connect(socket_path(profile))
        .map_err(|_| "WorktreeManager is not running for this profile. Open the app first.")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|_| "Cannot configure connection")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| "Cannot configure connection")?;
    let id = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let request = Request {
        id: id.clone(),
        version: VERSION,
        method: arguments.method,
        params: Value::Object(arguments.params),
    };
    let bytes = serde_json::to_vec(&request).map_err(|_| "Cannot encode request")?;
    if bytes.len() as u64 >= MAX_MESSAGE {
        return Err("Request exceeds 1 MiB".into());
    }
    stream
        .write_all(&bytes)
        .and_then(|_| stream.write_all(b"\n"))
        .map_err(|_| "Cannot send request")?;
    let mut reader = BufReader::new(stream);
    loop {
        let response = read_message(&mut reader)?;
        if response["id"] != id || response["version"] != VERSION {
            return Err("Unexpected response ID or protocol version".into());
        }
        if let Some(message) = response["progress"].as_str() {
            eprintln!("{message}");
        } else {
            return Ok(response);
        }
    }
}

fn application_command(executable: &Path) -> std::process::Command {
    #[cfg(target_os = "macos")]
    if let Some(contents) = executable.parent().and_then(Path::parent) {
        if contents.file_name().is_some_and(|name| name == "Contents") {
            if let Some(bundle) = contents
                .parent()
                .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
            {
                let mut command = std::process::Command::new("/usr/bin/open");
                command.arg("-a").arg(bundle);
                return command;
            }
        }
    }
    std::process::Command::new(executable)
}

fn open_application() -> Result<(), String> {
    let executable = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|_| "Cannot locate WorktreeManager.")?;
    let mut command = application_command(&executable);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if command.get_program() == "/usr/bin/open" {
        let status = command
            .status()
            .map_err(|_| "Could not open WorktreeManager.")?;
        if !status.success() {
            return Err("Could not open WorktreeManager.".into());
        }
    } else {
        command
            .spawn()
            .map_err(|_| "Could not start WorktreeManager.")?;
    }
    Ok(())
}

fn run_cli(
    executable: &str,
    mut args: Vec<String>,
    open: impl FnOnce() -> Result<(), String>,
) -> Option<i32> {
    let cli = Path::new(executable)
        .file_name()
        .is_some_and(|name| name == "wtm");
    if args.first().is_some_and(|arg| arg == "--cli") {
        args.remove(0);
    } else if !cli {
        return None;
    }
    if args.is_empty() {
        return Some(match open() {
            Ok(()) => 0,
            Err(error) => {
                println!("{}", failure("launch_failed", &error));
                1
            }
        });
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{HELP}");
        return Some(0);
    }
    if args == ["--version"] {
        println!("wtm {} (protocol {VERSION})", env!("CARGO_PKG_VERSION"));
        return Some(0);
    }
    let arguments = match parse(&args) {
        Ok(arguments) => arguments,
        Err(error) => {
            println!("{}", failure("invalid_args", &error));
            return Some(2);
        }
    };
    if arguments.method == "agent.event" {
        report_agent_event(arguments, read_hook_payload());
        return Some(0);
    }
    match execute(arguments) {
        Ok(response) => {
            let success = response["ok"] == true;
            println!("{response}");
            Some(if success { 0 } else { 1 })
        }
        Err(error) => {
            println!("{}", failure("client_error", &error));
            Some(1)
        }
    }
}

fn read_hook_payload() -> Value {
    if unsafe { libc::isatty(libc::STDIN_FILENO) } == 1 {
        return Value::Null;
    }
    let mut bytes = Vec::new();
    let _ = io::stdin().take(MAX_MESSAGE).read_to_end(&mut bytes);
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn agent_event_cwd(payload: &Value) -> Option<String> {
    if payload["stop_hook_active"] == true {
        return None;
    }
    payload["cwd"].as_str().map(str::to_owned).or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|dir| dir.to_string_lossy().into_owned())
    })
}

/// Which hook fired, where it matters: a submitted prompt answers any pending question, and
/// Codex's `request_user_input_async` asks without blocking, so its turn still ends with `Stop`.
fn agent_event_kind(payload: &Value) -> Map<String, Value> {
    let mut kind = Map::new();
    match payload["hook_event_name"].as_str() {
        Some("UserPromptSubmit") => {
            kind.insert("prompt".into(), json!(true));
        }
        Some("PreToolUse")
            if payload["tool_name"]
                .as_str()
                .is_some_and(|tool| tool.ends_with("request_user_input_async")) =>
        {
            kind.insert("asyncQuestion".into(), json!(true));
        }
        _ => {}
    }
    kind
}

/// Claude fires `Stop` while background subagents still run, and again once they report back, so
/// the turn only counts as done when none is left running.
fn agent_event_state(requested: &str, payload: &Value) -> String {
    let subagent_running = payload["background_tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|task| task["type"] == "subagent" && task["status"] == "running");
    if requested == "done" && subagent_running {
        "working".into()
    } else {
        requested.into()
    }
}

/// Embedded sessions export their task and surface; external ones are matched by cwd. `at`
/// orders events, since asynchronous hooks can be delivered out of order.
fn agent_event_origin(env: impl Fn(&str) -> Option<String>) -> Map<String, Value> {
    let mut origin = Map::new();
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    origin.insert("at".into(), json!(at));
    let task = env(agent_alerts::TASK_ENV).filter(|value| !value.is_empty());
    let surface = env(agent_alerts::SURFACE_ENV)
        .filter(|value| ["chat", "terminal", "editor"].contains(&value.as_str()));
    if let (Some(task), Some(surface)) = (task, surface) {
        origin.insert("taskId".into(), json!(task));
        origin.insert("surface".into(), json!(surface));
    }
    origin
}

fn report_agent_event(mut arguments: Arguments, payload: Value) {
    let origin = agent_event_origin(|key| std::env::var(key).ok());
    let Some(cwd) = agent_event_cwd(&payload) else {
        return;
    };
    let state = agent_event_state(
        arguments.params["state"].as_str().unwrap_or_default(),
        &payload,
    );
    arguments.params.insert("state".into(), json!(state));
    arguments.params.insert("cwd".into(), json!(cwd));
    arguments.params.extend(origin);
    arguments.params.extend(agent_event_kind(&payload));
    let delivered = execute(arguments).is_ok_and(|response| response["ok"] == true);
    if let Some(sound) = agent_alerts::default_sound(&state).filter(|_| !delivered) {
        agent_alerts::play_sound(sound);
    }
}

pub fn run_if_requested() -> Option<i32> {
    let mut args = std::env::args();
    let executable = args.next().unwrap_or_default();
    run_cli(&executable, args.collect(), open_application)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(input: &str) -> Vec<String> {
        input.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn bare_wtm_and_development_alias_open_the_app() {
        let mut launches = 0;
        assert_eq!(
            run_cli("/opt/homebrew/bin/wtm", vec![], || {
                launches += 1;
                Ok(())
            }),
            Some(0)
        );
        assert_eq!(
            run_cli("/build/worktree-manager", args("--cli"), || {
                launches += 1;
                Ok(())
            }),
            Some(0)
        );
        assert_eq!(launches, 2);
        assert_eq!(
            run_cli("wtm", vec![], || Err("launch failed".into())),
            Some(1)
        );
    }

    #[test]
    fn help_and_normal_app_start_do_not_launch_another_process() {
        assert_eq!(
            run_cli("wtm", args("--help"), || panic!("unexpected launch")),
            Some(0)
        );
        assert_eq!(
            run_cli("wtm", args("--version"), || panic!("unexpected launch")),
            Some(0)
        );
        assert_eq!(
            run_cli("/build/worktree-manager", vec![], || panic!(
                "unexpected launch"
            )),
            None
        );
        assert_eq!(
            run_cli("wtm", args("--local"), || panic!("unexpected launch")),
            Some(2)
        );
    }

    #[test]
    fn development_launch_uses_the_same_binary_without_cli_arguments() {
        let command = application_command(Path::new("/build/worktree-manager"));
        assert_eq!(command.get_program(), "/build/worktree-manager");
        assert_eq!(command.get_args().count(), 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bundled_launch_uses_launch_services_to_open_or_activate_the_exact_app() {
        let command = application_command(Path::new(
            "/Custom Apps/WorktreeManager.app/Contents/MacOS/worktree-manager",
        ));
        assert_eq!(command.get_program(), "/usr/bin/open");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["-a", "/Custom Apps/WorktreeManager.app"]
        );
    }
    #[test]
    fn parses_link_issue_and_requires_the_issue() {
        let parsed = parse(&args("--local task link-issue t1 --issue WOR-123")).unwrap();
        assert!(parsed.local);
        assert_eq!(parsed.method, "task.link-issue");
        assert_eq!(parsed.params["id"], "t1");
        assert_eq!(parsed.params["issue"], "WOR-123");
        for invalid in [
            "task link-issue t1",
            "task link-issue --issue WOR-123",
            "task link-issue t1 --issue",
            "task get t1 --issue WOR-123",
            "task link-issue t1 --issue A --issue B",
            "task link-issue t1 --issue A --input -",
        ] {
            assert!(parse(&args(invalid)).is_err(), "{invalid}");
        }
    }

    #[test]
    fn parses_agent_events_and_rejects_unknown_states() {
        let parsed = parse(&args("--local agent event done --agent codex")).unwrap();
        assert_eq!(parsed.method, "agent.event");
        assert_eq!(parsed.params["state"], "done");
        assert_eq!(parsed.params["agent"], "codex");
        assert!(!parsed.params.contains_key("id"));
        for invalid in [
            "agent event done",
            "agent event idle --agent claude",
            "agent event done --agent cursor",
            "agent event --agent claude",
        ] {
            assert!(parse(&args(invalid)).is_err(), "{invalid}");
        }
    }

    #[test]
    fn agent_event_origin_needs_both_task_and_a_known_surface() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |key: &str| {
                pairs
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value.to_string())
            }
        };
        let origin = agent_event_origin(env(&[("WTM_TASK_ID", "t1"), ("WTM_SURFACE", "editor")]));
        assert_eq!(origin["taskId"], "t1");
        assert_eq!(origin["surface"], "editor");
        assert!(origin["at"].as_u64().unwrap() > 0);
        for pairs in [
            &[("WTM_TASK_ID", "t1")][..],
            &[("WTM_TASK_ID", "t1"), ("WTM_SURFACE", "cursor")][..],
            &[("WTM_TASK_ID", ""), ("WTM_SURFACE", "chat")][..],
        ] {
            let origin = agent_event_origin(env(pairs));
            assert!(!origin.contains_key("taskId") && !origin.contains_key("surface"));
        }
    }

    #[test]
    fn agent_event_kind_flags_prompts_and_async_questions() {
        assert_eq!(
            agent_event_kind(&json!({ "hook_event_name": "UserPromptSubmit" }))["prompt"],
            true
        );
        assert_eq!(
            agent_event_kind(&json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "request_user_input_async"
            }))["asyncQuestion"],
            true
        );
        for payload in [
            json!({ "hook_event_name": "PreToolUse", "tool_name": "request_user_input" }),
            json!({ "hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion" }),
            json!({ "hook_event_name": "Stop" }),
            Value::Null,
        ] {
            assert!(agent_event_kind(&payload).is_empty(), "{payload}");
        }
    }

    #[test]
    fn stop_with_running_subagents_keeps_the_agent_working() {
        let running = json!({ "background_tasks": [
            { "id": "a1", "type": "subagent", "status": "running" }
        ] });
        assert_eq!(agent_event_state("done", &running), "working");
        assert_eq!(agent_event_state("waiting", &running), "waiting");
        for payload in [
            json!({ "background_tasks": [] }),
            json!({ "background_tasks": [{ "type": "subagent", "status": "completed" }] }),
            json!({ "background_tasks": [{ "type": "shell", "status": "running" }] }),
            Value::Null,
        ] {
            assert_eq!(agent_event_state("done", &payload), "done", "{payload}");
        }
    }

    #[test]
    fn agent_event_cwd_comes_from_the_hook_payload() {
        assert_eq!(
            agent_event_cwd(&json!({ "cwd": "/wt/repo/task" })).as_deref(),
            Some("/wt/repo/task")
        );
        assert!(agent_event_cwd(&json!({ "cwd": "/wt", "stop_hook_active": true })).is_none());
        assert!(agent_event_cwd(&Value::Null).is_some());
    }

    #[test]
    fn parses_supported_commands_and_rejects_ambiguous_deletion() {
        assert_eq!(
            parse(&args("--local task list --workspace w1"))
                .unwrap()
                .params["workspaceId"],
            "w1"
        );
        assert!(parse(&args("task update t1 --input -")).is_err());
        assert!(parse(&args("task delete t1")).is_err());
        assert!(parse(&args("task delete t1 --keep-worktrees --delete-worktrees")).is_err());
        assert!(parse(&args("task delete t1 --keep-worktrees --force")).is_err());
        assert!(parse(&args("workspace list --input -")).is_err());
        assert!(parse(&args("task get t1 --git")).is_ok());
        assert!(parse(&args("workspace update w1 --input -")).is_ok());
    }
}
