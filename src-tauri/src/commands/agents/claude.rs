//! Claude Code. Chat runs headless over stream-json with one session id per conversation;
//! forks relaunch Claude on a new session cut at a message. The embedded terminal keeps a named
//! session per worktree (`-n wm-<slug>`), continued with `-c` on later launches, `/color` for a
//! distinguishing window color and `--add-dir` for the task's extra repos. Past conversations are
//! read from the transcripts under `~/.claude/projects`; renames go through `rename_session` on a
//! resumed headless session. Transcripts are never deleted.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use serde_json::{json, Value};

use super::permission::{PermissionMode, PermissionPolicy};
use super::{
    AgentAdapter, AgentCapabilities, ChatRequest, ConversationFork, ForkLaunch, ForkMethod,
    ForkRequest, LaunchContext, LaunchSpec,
};
use crate::commands::agent_alerts::{AlertHooks, HookSpec};
use crate::commands::agent_usage::UsageSource;
use crate::commands::chat::claude::{
    fork_point, history, launch_args, projects_dir, session_exists, ClaudeProvider,
    FORWARD_SUBAGENT_TEXT_ENV, HEADLESS_ARGS,
};
use crate::commands::chat::{
    exchange, truncate, ChatItem, ItemKind, Launch, SessionList, SessionSummary, LIST_LIMIT,
};
use crate::commands::claude_usage::ClaudeUsage;
use crate::commands::login_shell::{cli_available, LoginShell};

/// Env var holding the per-worktree session name. Long, prefixed name avoids clashing with
/// anything a user might already export in their shell.
const SESSION_ENV: &str = "WORKTREE_MANAGER_CLAUDE_SESSION_NAME";
/// Forks cut the transcript at a message through flags first verified on this release.
const MIN_FORK_VERSION: (u32, u32, u32) = (2, 1, 295);

const ALERT_HOOKS: &[HookSpec] = &[
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

pub struct Claude {
    program: String,
    home: Option<PathBuf>,
}

impl Claude {
    pub(crate) fn new(program: impl Into<String>, home: Option<PathBuf>) -> Self {
        Self {
            program: program.into(),
            home,
        }
    }

    pub(crate) fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    fn require_cli(&self) -> Result<(), String> {
        if self.installed() {
            Ok(())
        } else {
            Err("claude CLI not found on PATH (see Doctor)".into())
        }
    }

    fn session_exists(&self, cwd: &str, id: &str) -> bool {
        session_exists(self.home(), cwd, id)
    }

    fn fork(&self, request: &ForkRequest, source: &str) -> Result<Launch, String> {
        let version = self
            .cli(request.cwd, ["--version"])
            .output()
            .map_err(|e| e.to_string())?;
        if !version.status.success() || !supports_fork(&String::from_utf8_lossy(&version.stdout)) {
            let (major, minor, patch) = MIN_FORK_VERSION;
            return Err(format!(
                "Forking requires the verified Claude Code {major}.{minor}.{patch} or newer"
            ));
        }
        let (anchor, history) = fork_point(self.home(), request.cwd, source, request.item_id)?;
        let permission = permission_args(&PermissionPolicy::Interactive);
        let args = if let Some(anchor) = anchor {
            let mut args = launch_args(source, true, request.extra_dirs, &permission);
            args.extend([
                "--fork-session".into(),
                "--session-id".into(),
                request.fork_id.into(),
                "--resume-session-at".into(),
                anchor,
            ]);
            args
        } else {
            launch_args(request.fork_id, false, request.extra_dirs, &permission)
        };
        let mut provider = ClaudeProvider::new(request.fork_id.to_string()).forking();
        if !history.iter().any(|item| item.kind == ItemKind::User) {
            provider = provider.continuing(request.handoff.map(String::from));
        }
        Ok(Launch {
            command: self.chat_cli(request.cwd, &args).command(),
            provider: Box::new(provider),
            history,
        })
    }

    fn cli<I, S>(&self, cwd: &str, args: I) -> LoginShell
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        LoginShell::program(&self.program, args).current_dir(cwd)
    }

    fn chat_cli<I, S>(&self, cwd: &str, args: I) -> LoginShell
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.cli(cwd, args).env(FORWARD_SUBAGENT_TEXT_ENV, "1")
    }

    fn headless(&self, cwd: &str, args: &[&str]) -> std::process::Command {
        self.cli(cwd, HEADLESS_ARGS.iter().chain(args)).command()
    }

    /// Whether the installed Claude Code accepts `rename_session`, probed on a throwaway session
    /// in a scratch directory that never sends a message. Only success is cached, so a timeout
    /// retries.
    fn can_rename(&self) -> bool {
        static SUPPORTED: AtomicBool = AtomicBool::new(false);
        if SUPPORTED.load(Ordering::Relaxed) {
            return true;
        }
        let supported = self.probe_rename().unwrap_or(false);
        if supported {
            SUPPORTED.store(true, Ordering::Relaxed);
        }
        supported
    }

    fn probe_rename(&self) -> Option<bool> {
        let scratch =
            std::env::temp_dir().join(format!("wm-claude-probe-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&scratch).ok()?;
        let cwd = fs::canonicalize(&scratch).unwrap_or(scratch.clone());
        let cwd = cwd.to_string_lossy();
        let probe = uuid::Uuid::new_v4().to_string();
        let supported = exchange(
            self.headless(&cwd, &["--session-id", &probe]),
            &rename_lines("probe"),
            rename_reply,
        )
        .is_ok();
        if let Some(dir) = projects_dir(self.home(), &cwd) {
            let _ = fs::remove_file(dir.join(format!("{probe}.jsonl")));
            let _ = fs::remove_dir(dir);
        }
        let _ = fs::remove_dir_all(&scratch);
        Some(supported)
    }
}

impl AgentAdapter for Claude {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn label(&self) -> &'static str {
        "Claude Code"
    }

    fn installed(&self) -> bool {
        cli_available(&self.program)
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            fork: Some(self.fork_method()),
            review: false,
            rename: true,
            archive: false,
            models: true,
            effort: true,
            permission_modes: &[PermissionMode::Interactive, PermissionMode::Unattended],
            interrupt: true,
            steer: false,
            compact: true,
            images: true,
            headless: true,
            tool_rules: true,
        }
    }

    fn chat_launch(&self, request: ChatRequest) -> Result<Launch, String> {
        self.require_cli()?;
        let cwd = request.cwd.as_str();
        if let (Some(id), Some(fork)) = (&request.resume, &request.fork) {
            if !self.session_exists(cwd, id) {
                let fork_request = ForkRequest {
                    cwd,
                    extra_dirs: &request.extra_dirs,
                    fork_id: id,
                    source: &fork.source,
                    pending_source: None,
                    item_id: &fork.item_id,
                    handoff: request.handoff.as_deref(),
                };
                return self.fork(&fork_request, &fork.source);
            }
        }
        let resume = resume_target(request.resume, request.pinned, |id| {
            self.session_exists(cwd, id)
        })?;
        let history = resume
            .as_deref()
            .map(|id| history(self.home(), cwd, id))
            .unwrap_or_default();
        let session_id = resume
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Ok(Launch {
            command: self
                .chat_cli(
                    cwd,
                    launch_args(
                        &session_id,
                        resume.is_some(),
                        &request.extra_dirs,
                        &permission_args(&request.permission),
                    ),
                )
                .command(),
            provider: Box::new(
                ClaudeProvider::new(session_id)
                    .continuing(request.handoff.filter(|_| resume.is_none())),
            ),
            history,
        })
    }

    fn fork_method(&self) -> ForkMethod {
        ForkMethod::Relaunch
    }

    fn fork_launch(&self, request: ForkRequest) -> Result<ForkLaunch, String> {
        let source = if self.session_exists(request.cwd, request.source) {
            request.source
        } else {
            request.pending_source.unwrap_or(request.source)
        }
        .to_string();
        Ok(ForkLaunch {
            launch: self.fork(&request, &source)?,
            source,
        })
    }

    fn terminal_launch(&self, ctx: &LaunchContext) -> Result<LaunchSpec, String> {
        self.require_cli()?;
        let slug = branch_to_session_slug(ctx.branch_name, ctx.canonical_dir);
        let resume = marker_matches(ctx, &slug);
        let spec = terminal_session(&self.program, &slug, resume, ctx.extra_dirs)
            .current_dir(ctx.canonical_dir)
            .env(SESSION_ENV, &slug)
            .terminal();
        if !resume {
            write_marker(ctx, &slug)?;
        }
        Ok(spec)
    }

    fn forget_terminal_session(&self, ctx: &LaunchContext) {
        let _ = fs::remove_file(marker_path(ctx.session_store, ctx.canonical_dir));
    }

    fn sessions(&self, cwd: &str, current: Option<&str>) -> Result<SessionList, String> {
        Ok(SessionList {
            sessions: projects_dir(self.home(), cwd)
                .map(|dir| transcripts(&dir, current))
                .unwrap_or_default(),
            can_rename: self.can_rename(),
            can_archive: false,
        })
    }

    fn history(
        &self,
        cwd: &str,
        id: &str,
        fork: Option<&ConversationFork>,
    ) -> Result<Vec<ChatItem>, String> {
        match fork {
            Some(fork) if !self.session_exists(cwd, id) => {
                fork_point(self.home(), cwd, &fork.source, &fork.item_id)
                    .map(|(_, history)| history)
            }
            _ => Ok(history(self.home(), cwd, id)),
        }
    }

    fn rename_session(&self, cwd: &str, id: &str, title: &str) -> Result<(), String> {
        exchange(
            self.headless(cwd, &["--resume", id]),
            &rename_lines(title),
            rename_reply,
        )
        .map(|_| ())
    }

    fn live_session_renames_itself(&self) -> bool {
        true
    }

    fn alert_hooks(&self) -> AlertHooks {
        AlertHooks {
            agent: "claude",
            config: self
                .home
                .clone()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".claude")
                .join("settings.json"),
            specs: ALERT_HOOKS,
            async_handlers: true,
        }
    }

    fn usage_source(&self) -> Box<dyn UsageSource> {
        Box::new(ClaudeUsage::new(
            self.home.as_ref().map(|home| home.join(".claude.json")),
        ))
    }
}

fn resume_target(
    resume: Option<String>,
    pinned: bool,
    exists: impl Fn(&str) -> bool,
) -> Result<Option<String>, String> {
    match resume {
        Some(id) if !exists(&id) && pinned => Err("That conversation no longer exists".into()),
        resume => Ok(resume.filter(|id| exists(id))),
    }
}

fn transcript_title(text: &str) -> Option<String> {
    let (mut custom, mut generated, mut first) = (None, None, None);
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match entry["type"].as_str() {
            Some("custom-title") => custom = entry["customTitle"].as_str().map(String::from),
            Some("ai-title") => generated = entry["aiTitle"].as_str().map(String::from),
            Some("user")
                if first.is_none() && entry["isSidechain"] != true && entry["isMeta"] != true =>
            {
                first = user_text(&entry["message"]["content"]);
            }
            _ => {}
        }
    }
    let title = custom.or(generated);
    first.as_ref()?;
    title.or(first).map(|title| truncate(&title))
}

fn user_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(text) => Some(text.as_str()),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .find(|text| !text.starts_with('<')),
        _ => None,
    }?;
    (!text.starts_with('<') && !text.trim().is_empty()).then(|| text.to_string())
}

fn transcripts(dir: &Path, current: Option<&str>) -> Vec<SessionSummary> {
    let mut sessions: Vec<SessionSummary> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|path| {
            let id = path.file_stem()?.to_str()?.to_string();
            let title = transcript_title(&fs::read_to_string(&path).ok()?)?;
            let updated_at = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_secs());
            Some(SessionSummary {
                current: current == Some(id.as_str()),
                id,
                title,
                updated_at,
            })
        })
        .collect();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
    sessions.truncate(LIST_LIMIT);
    sessions
}

fn rename_lines(title: &str) -> [Value; 2] {
    [
        json!({ "type": "control_request", "request_id": "init", "request": { "subtype": "initialize" } }),
        json!({ "type": "control_request", "request_id": "rename", "request": { "subtype": "rename_session", "title": title } }),
    ]
}

fn rename_reply(message: &Value) -> Option<Result<Value, String>> {
    let response = &message["response"];
    if message["type"] != "control_response" || response["request_id"] != "rename" {
        return None;
    }
    Some(match response["subtype"].as_str() {
        Some("success") => Ok(Value::Null),
        _ => Err(response["error"]
            .as_str()
            .unwrap_or("Claude could not rename the session")
            .to_string()),
    })
}

/// `-c` is directory-scoped and never prompts, unlike `--resume <name>` which opens an
/// interactive picker. A fast non-zero exit from `-c` means there was nothing to continue
/// (e.g. the session was closed before Claude persisted anything) — fall back to a fresh named
/// session. A slow non-zero exit is a real session that later failed, so propagate it.
const CONTINUE_OR_START_NAMED: &str = r#"program=$1 slug=$2
shift 2
s=$SECONDS
"$program" -c "$@"
rc=$?
if [ $rc -ne 0 ] && [ $((SECONDS - s)) -lt 10 ]; then exec "$program" -n "$slug" /color "$@"; fi
exit $rc"#;

/// `/color` must precede the variadic `--add-dir` flags, which would otherwise swallow it.
fn terminal_session(program: &str, slug: &str, resume: bool, extra_dirs: &[String]) -> LoginShell {
    let add_dirs = extra_dirs
        .iter()
        .flat_map(|dir| ["--add-dir", dir.as_str()]);
    if resume {
        LoginShell::script(CONTINUE_OR_START_NAMED)
            .args([program, slug])
            .args(add_dirs)
    } else {
        LoginShell::program(program, ["-n", slug, "/color"]).args(add_dirs)
    }
}

fn marker_path(session_store: &Path, canonical_dir: &str) -> PathBuf {
    session_store
        .join("claude")
        .join(format!("{:x}.json", md5::compute(canonical_dir)))
}

fn marker_matches(ctx: &LaunchContext, slug: &str) -> bool {
    fs::read_to_string(marker_path(ctx.session_store, ctx.canonical_dir))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("slug").and_then(|s| s.as_str()).map(|s| s == slug))
        .unwrap_or(false)
}

fn write_marker(ctx: &LaunchContext, slug: &str) -> Result<(), String> {
    let path = marker_path(ctx.session_store, ctx.canonical_dir);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let body = serde_json::json!({ "slug": slug, "canonicalDir": ctx.canonical_dir });
    fs::write(&path, body.to_string()).map_err(|e| format!("write session marker: {e}"))
}

/// Interactive sessions let Claude's classifier approve what it can and ask about the rest.
/// Unattended ones never prompt: only the allowed tools run without asking, and every other
/// request is denied.
fn permission_args(policy: &PermissionPolicy) -> Vec<String> {
    let PermissionPolicy::Unattended(policy) = policy else {
        return [
            "--permission-mode",
            "auto",
            "--allow-dangerously-skip-permissions",
        ]
        .map(String::from)
        .to_vec();
    };
    let mut args = vec!["--permission-mode".to_string(), "dontAsk".to_string()];
    for (flag, rules) in [
        ("--allowedTools", &policy.allow),
        ("--disallowedTools", &policy.deny),
    ] {
        if !rules.is_empty() {
            args.push(flag.to_string());
            args.extend(rules.iter().cloned());
        }
    }
    args
}

/// Derive `wm-…` session slug from branch name, with path fallback.
pub fn branch_to_session_slug(branch_input: Option<&str>, path_fallback: &str) -> String {
    let trimmed = branch_input.map(str::trim).unwrap_or("");
    let base = if !trimmed.is_empty() {
        trimmed
    } else {
        Path::new(path_fallback)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("worktree")
    };

    let mut s: String = base
        .chars()
        .map(|c| {
            if c == '/' || c.is_whitespace() {
                '-'
            } else if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    while s.contains("--") {
        s = s.replace("--", "-");
    }
    s = s.trim_matches('-').to_string();
    let mut out = format!("wm-{s}");
    if out.len() > 64 {
        out.truncate(64);
        while out.ends_with('-') {
            out.pop();
        }
    }
    if out == "wm-" || out.is_empty() {
        let mut h = DefaultHasher::new();
        path_fallback.hash(&mut h);
        out = format!("wm-{:x}", h.finish());
    }
    out
}

fn supports_fork(version_output: &str) -> bool {
    cli_version(version_output).is_some_and(|version| version >= MIN_FORK_VERSION)
}

fn cli_version(version_output: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version_output
        .split_whitespace()
        .next()?
        .splitn(3, '.')
        .map(|part| part.parse().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agents::permission::UnattendedPolicy;
    use std::os::unix::fs::PermissionsExt;

    fn temp_store(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wm-agent-claude-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_claude(dir: &Path, continue_exit: i32) -> String {
        let program = dir.join("fake-claude");
        fs::write(
            &program,
            format!(
                "#!/bin/sh\nif [ \"$1\" = -c ]; then echo \"continued $*\"; exit {continue_exit}; fi\necho \"named $*\"\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        program.to_string_lossy().into_owned()
    }

    fn terminal_output(launch: LoginShell, home: &Path) -> (Option<i32>, String) {
        let output = launch
            .command()
            .env("HOME", home)
            .env_remove("ZDOTDIR")
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    #[test]
    fn fork_is_supported_from_the_minimum_version_on() {
        assert!(supports_fork("2.1.295 (Claude Code)\n"));
        assert!(supports_fork("2.1.296 (Claude Code)"));
        assert!(supports_fork("2.2.0 (Claude Code)"));
        assert!(supports_fork("3.0.0 (Claude Code)"));
        assert!(!supports_fork("2.1.294 (Claude Code)"));
        assert!(!supports_fork("2.0.999 (Claude Code)"));
        assert!(!supports_fork("1.9.400 (Claude Code)"));
    }

    #[test]
    fn fork_is_unsupported_when_the_version_is_unparseable() {
        for output in ["", "Claude Code", "2.1", "2.1.x (Claude Code)", "v2.1.295"] {
            assert!(!supports_fork(output), "{output}");
        }
    }

    #[test]
    fn named_session_has_color_before_add_dirs_and_never_resume_flag() {
        let extra = vec!["/tmp/a b".to_string(), "/tmp/c".to_string()];
        assert_eq!(
            terminal_session("claude", "wm-x", false, &extra),
            LoginShell::program(
                "claude",
                [
                    "-n",
                    "wm-x",
                    "/color",
                    "--add-dir",
                    "/tmp/a b",
                    "--add-dir",
                    "/tmp/c"
                ]
            )
        );
    }

    #[test]
    fn resume_continues_and_falls_back_to_the_named_session_only_on_fast_failure() {
        let store = temp_store("resume");
        let extra = ["/tmp/a b".to_string()];
        let continued = fake_claude(&store, 0);
        assert_eq!(
            terminal_output(terminal_session(&continued, "wm-x", true, &extra), &store),
            (Some(0), "continued -c --add-dir /tmp/a b\n".to_string())
        );
        let missing = fake_claude(&store, 3);
        assert_eq!(
            terminal_output(terminal_session(&missing, "wm-x", true, &extra), &store),
            (
                Some(0),
                "continued -c --add-dir /tmp/a b\nnamed -n wm-x /color --add-dir /tmp/a b\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn terminal_launch_starts_a_named_session_then_continues_it() {
        let store = temp_store("terminal-launch");
        let claude = Claude::new(fake_claude(&store, 0), None);
        let ctx = LaunchContext {
            canonical_dir: "/tmp",
            extra_dirs: &[],
            branch_name: Some("feat/x"),
            session_store: &store,
        };
        let first = claude.terminal_launch(&ctx).unwrap();
        assert_eq!(first.cwd, "/tmp");
        assert_eq!(
            first.env,
            vec![(SESSION_ENV.to_string(), "wm-feat-x".to_string())]
        );
        assert!(first
            .args
            .ends_with(&["-n".into(), "wm-feat-x".into(), "/color".into()]));
        let second = claude.terminal_launch(&ctx).unwrap();
        assert!(second
            .args
            .ends_with(&[claude.program.clone(), "wm-feat-x".into()]));
    }

    #[test]
    fn marker_roundtrip_switches_resume() {
        let store = temp_store("marker");
        let ctx = LaunchContext {
            canonical_dir: "/tmp/wt",
            extra_dirs: &[],
            branch_name: Some("feat/x"),
            session_store: &store,
        };
        assert!(!marker_matches(&ctx, "wm-feat-x"));
        write_marker(&ctx, "wm-feat-x").unwrap();
        assert!(marker_matches(&ctx, "wm-feat-x"));
        assert!(!marker_matches(&ctx, "wm-other"));
        Claude::new("claude", None).forget_terminal_session(&ctx);
        assert!(!marker_matches(&ctx, "wm-feat-x"));
        let _ = fs::remove_dir_all(&store);
    }

    #[test]
    fn marker_path_is_keyed_by_canonical_dir() {
        let store = Path::new("/store");
        assert_ne!(marker_path(store, "/a"), marker_path(store, "/b"));
        assert_eq!(marker_path(store, "/a"), marker_path(store, "/a"));
        assert!(marker_path(store, "/a").starts_with("/store/claude"));
    }

    #[test]
    fn slug_normalizes_branch_names() {
        assert_eq!(
            branch_to_session_slug(Some("Feat/My Thing!"), "/x"),
            "wm-feat-my-thing"
        );
        assert_eq!(branch_to_session_slug(Some("a__b--c"), "/x"), "wm-a__b-c");
    }

    #[test]
    fn slug_falls_back_to_path_then_hash() {
        assert_eq!(
            branch_to_session_slug(None, "/repos/worktrees/wt-1"),
            "wm-wt-1"
        );
        assert_eq!(branch_to_session_slug(Some("   "), "/repos/wt2"), "wm-wt2");
        let hashed = branch_to_session_slug(Some("///"), "");
        assert!(hashed.starts_with("wm-") && hashed.len() > 3);
    }

    #[test]
    fn slug_is_capped_at_64_without_trailing_dash() {
        let long = "a-".repeat(60);
        let s = branch_to_session_slug(Some(&long), "/x");
        assert!(s.len() <= 64);
        assert!(!s.ends_with('-'));
    }

    #[test]
    fn picked_conversations_must_exist() {
        let exists = |id: &str| id == "known";
        assert!(resume_target(Some("gone".into()), true, exists).is_err());
        assert_eq!(resume_target(Some("gone".into()), false, exists), Ok(None));
        assert_eq!(
            resume_target(Some("known".into()), true, exists),
            Ok(Some("known".into()))
        );
    }

    #[test]
    fn transcript_title_prefers_custom_then_generated_then_first_message() {
        let user = r#"not json
{"type":"user","message":{"content":"<command>x</command>"}}
{"type":"user","message":{"content":[{"type":"text","text":"Refactor the store"}]}}"#;
        assert_eq!(
            transcript_title(user).as_deref(),
            Some("Refactor the store")
        );
        let generated = format!(
            "{user}\n{}",
            r#"{"type":"ai-title","aiTitle":"Store refactor"}"#
        );
        assert_eq!(
            transcript_title(&generated).as_deref(),
            Some("Store refactor")
        );
        let custom = format!(
            "{generated}\n{}",
            r#"{"type":"custom-title","customTitle":"Mine"}"#
        );
        assert_eq!(transcript_title(&custom).as_deref(), Some("Mine"));
        assert_eq!(user_text(&json!(42)), None);
    }

    #[test]
    fn transcripts_skip_files_without_a_user_message() {
        let dir = temp_store("sessions");
        fs::write(
            dir.join("a.jsonl"),
            r#"{"type":"user","message":{"content":"Hello"}}"#,
        )
        .unwrap();
        fs::write(dir.join("b.jsonl"), r#"{"type":"queue-operation"}"#).unwrap();
        fs::write(dir.join("notes.txt"), "Hello").unwrap();
        let sessions = transcripts(&dir, Some("a"));
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            (
                sessions[0].id.as_str(),
                sessions[0].title.as_str(),
                sessions[0].current
            ),
            ("a", "Hello", true)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_reply_surfaces_claude_errors() {
        let ok = json!({ "type": "control_response", "response": { "subtype": "success", "request_id": "rename" } });
        let failed = json!({ "type": "control_response", "response": { "subtype": "error", "request_id": "rename", "error": "nope" } });
        let other = json!({ "type": "control_response", "response": { "subtype": "success", "request_id": "init" } });
        assert_eq!(rename_reply(&ok), Some(Ok(Value::Null)));
        assert_eq!(rename_reply(&failed), Some(Err("nope".into())));
        assert_eq!(rename_reply(&other), None);
    }

    #[test]
    fn alert_hooks_and_usage_live_in_the_claude_home() {
        let claude = Claude::new("claude", Some(PathBuf::from("/home/me")));
        let hooks = claude.alert_hooks();
        assert_eq!(hooks.agent, "claude");
        assert_eq!(hooks.config, Path::new("/home/me/.claude/settings.json"));
        assert!(hooks.async_handlers);
        assert_eq!(hooks.specs.len(), 5);
        assert_eq!(
            Claude::new("claude", None).alert_hooks().config,
            Path::new("/tmp/.claude/settings.json")
        );
        assert!(claude.live_session_renames_itself());
        assert_eq!(claude.fork_method(), ForkMethod::Relaunch);
        assert!(claude.archive_session("/tmp", "id").is_err());
    }

    #[test]
    fn interactive_sessions_keep_the_classifier_and_ask_for_the_rest() {
        assert_eq!(
            permission_args(&PermissionPolicy::Interactive),
            [
                "--permission-mode",
                "auto",
                "--allow-dangerously-skip-permissions"
            ]
        );
    }

    #[test]
    fn unattended_sessions_never_prompt_and_pass_their_tool_rules() {
        let policy = |allow: &[&str], deny: &[&str]| {
            PermissionPolicy::Unattended(UnattendedPolicy {
                allow: allow.iter().map(|rule| rule.to_string()).collect(),
                deny: deny.iter().map(|rule| rule.to_string()).collect(),
                ..Default::default()
            })
        };
        assert_eq!(
            permission_args(&policy(&[], &[])),
            ["--permission-mode", "dontAsk"]
        );
        assert_eq!(
            permission_args(&policy(&["Bash(git commit *)", "Edit"], &["WebFetch"])),
            [
                "--permission-mode",
                "dontAsk",
                "--allowedTools",
                "Bash(git commit *)",
                "Edit",
                "--disallowedTools",
                "WebFetch"
            ]
        );
        assert_eq!(
            permission_args(&policy(&[], &["Bash(rm *)"])),
            [
                "--permission-mode",
                "dontAsk",
                "--disallowedTools",
                "Bash(rm *)"
            ]
        );
    }

    #[test]
    fn missing_cli_fails_every_launch() {
        let claude = Claude::new("/missing-test-agent", None);
        let ctx = LaunchContext {
            canonical_dir: "/tmp/wt",
            extra_dirs: &[],
            branch_name: None,
            session_store: Path::new("/tmp/unused"),
        };
        assert!(!claude.installed());
        assert!(claude.terminal_launch(&ctx).is_err());
        assert!(claude
            .chat_launch(ChatRequest {
                cwd: "/tmp".into(),
                extra_dirs: Vec::new(),
                resume: None,
                pinned: false,
                fork: None,
                handoff: None,
                permission: PermissionPolicy::Interactive,
            })
            .is_err());
    }
}
