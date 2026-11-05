//! GitHub and Git as the watchers read them: `gh` and fetches go through the login shell so its
//! auth and credential helpers see the user's environment, each bounded by a timeout.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};

use crate::commands::git::git_command;
use crate::commands::github::run_process;
use crate::commands::login_shell::LoginShell;

const GH_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);

pub trait WatchForge: Send + Sync {
    /// GitHub's answer to a GraphQL query, partial data included.
    fn graphql(&self, query: &str, variables: Value) -> Result<Value, String>;
    /// The URL of the newest PR whose head is `branch`, in the repository of `worktree`.
    fn branch_pull_request(&self, worktree: &Path, branch: &str) -> Result<Option<String>, String>;
    fn comment(&self, url: &str, body: &str) -> Result<(), String>;
    /// Whether the changes of PR `number`, at `head`, are on `base` as `origin` has it now: its
    /// head is an ancestor of the base, or merging it into the base changes nothing.
    fn integrated(
        &self,
        worktree: &Path,
        number: u64,
        head: &str,
        base: &str,
    ) -> Result<bool, String>;
}

pub struct CliWatchForge {
    gh: String,
}

impl Default for CliWatchForge {
    fn default() -> Self {
        Self::with("gh")
    }
}

impl CliWatchForge {
    /// `gh` is the program GitHub calls run.
    pub(crate) fn with(gh: &str) -> Self {
        Self { gh: gh.to_string() }
    }

    fn gh(&self, args: &[&str], input: &[u8], worktree: Option<&Path>) -> Result<Vec<u8>, String> {
        let mut shell = LoginShell::program(&self.gh, args);
        if let Some(worktree) = worktree {
            shell = shell.current_dir(worktree);
        }
        let partial = args.first() == Some(&"api");
        run_process(&mut shell.command(), input, GH_TIMEOUT, partial)
    }
}

impl WatchForge for CliWatchForge {
    fn graphql(&self, query: &str, variables: Value) -> Result<Value, String> {
        let payload = json!({ "query": query, "variables": variables });
        let args = ["api", "graphql", "--hostname", "github.com", "--input", "-"];
        let answer = self.gh(&args, payload.to_string().as_bytes(), None)?;
        serde_json::from_slice(&answer).map_err(|_| "Invalid GitHub response".to_string())
    }

    fn branch_pull_request(&self, worktree: &Path, branch: &str) -> Result<Option<String>, String> {
        let args = [
            "pr", "list", "--head", branch, "--state", "all", "--limit", "1", "--json", "url",
        ];
        let listed: Value = serde_json::from_slice(&self.gh(&args, &[], Some(worktree))?)
            .map_err(|_| "Invalid GitHub response".to_string())?;
        Ok(listed[0]["url"].as_str().map(String::from))
    }

    fn comment(&self, url: &str, body: &str) -> Result<(), String> {
        self.gh(&["pr", "comment", url, "--body", body], &[], None)
            .map(|_| ())
    }

    fn integrated(
        &self,
        worktree: &Path,
        number: u64,
        head: &str,
        base: &str,
    ) -> Result<bool, String> {
        let base_ref = format!("refs/heads/{base}");
        let head_ref = format!("refs/pull/{number}/head");
        let directory = worktree.to_string_lossy();
        let fetch = [
            "-C",
            &directory,
            "fetch",
            "--quiet",
            "--no-tags",
            "origin",
            &base_ref,
            &head_ref,
        ];
        run_process(
            &mut LoginShell::program("git", fetch).command(),
            &[],
            FETCH_TIMEOUT,
            false,
        )
        .map_err(|_| format!("Could not fetch {base} and PR #{number} from origin"))?;
        let base_head = git(worktree, &["rev-parse", "FETCH_HEAD"])?;
        if git_status(worktree, &["merge-base", "--is-ancestor", head, &base_head])? {
            return Ok(true);
        }
        let merged = local_git(worktree, &["merge-tree", "--write-tree", &base_head, head])
            .output()
            .map_err(|error| format!("git merge-tree: {error}"))?;
        let merged_tree = String::from_utf8_lossy(&merged.stdout);
        let base_tree = git(worktree, &["rev-parse", &format!("{base_head}^{{tree}}")])?;
        Ok(merged.status.success() && merged_tree.lines().next() == Some(base_tree.as_str()))
    }
}

fn local_git(worktree: &Path, args: &[&str]) -> Command {
    let mut command = git_command();
    command.arg("-C").arg(worktree).args(args);
    command
}

pub(super) fn git(worktree: &Path, args: &[&str]) -> Result<String, String> {
    let output = local_git(worktree, args)
        .output()
        .map_err(|error| format!("git {}: {error}", args[0]))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {}: {}", args[0], stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Whether a git command that answers with its exit code said yes.
fn git_status(worktree: &Path, args: &[&str]) -> Result<bool, String> {
    let output = local_git(worktree, args)
        .output()
        .map_err(|error| format!("git {}: {error}", args[0]))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git {}: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}
