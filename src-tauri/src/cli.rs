use crate::automation::{failure, read_message, socket_path, Request, MAX_MESSAGE, VERSION};
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
