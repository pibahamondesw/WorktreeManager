//! Git and `gh` as the owner runs them: pushes and GitHub calls go through the login shell so
//! hooks, credential helpers and `gh` auth see the user's environment.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::super::selector::{OpenPullRequest, OpenPullRequests};
use super::{Forge, NewPullRequest, PullRequest};
use crate::commands::git::git_command;
use crate::commands::login_shell::LoginShell;
use crate::commands::terminal::run_headless;

const HEADLESS_SESSION: &str = "automation-publish";
const PR_FIELDS: &str = "number,url,title,body,isDraft";
/// The end of a failed command's output kept as the cause.
const OUTPUT_TAIL: usize = 4 * 1024;

pub struct CliForge {
    gh: String,
    env: Vec<(String, String)>,
}

impl Default for CliForge {
    fn default() -> Self {
        Self::with("gh", Vec::new())
    }
}

impl CliForge {
    /// `gh` is the program GitHub calls run; `env` reaches every command.
    pub(crate) fn with(gh: &str, env: Vec<(String, String)>) -> Self {
        Self {
            gh: gh.to_string(),
            env,
        }
    }

    fn login(&self, program: &str, args: &[&str], worktree: &Path) -> LoginShell {
        LoginShell::program(program, args)
            .current_dir(worktree)
            .envs(self.env.iter().cloned())
    }

    fn gh(&self, worktree: &Path, args: &[&str]) -> Result<String, String> {
        let output = self
            .login(&self.gh, args, worktree)
            .output()
            .map_err(|e| format!("gh {}: {e}", args[0]))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "gh {} {}: {}",
                args[0],
                args[1],
                tail(stderr.trim())
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn git(&self, worktree: &Path, args: &[&str]) -> Result<String, String> {
        let output = git_command()
            .arg("-C")
            .arg(worktree)
            .args(args)
            .envs(self.env.iter().cloned())
            .output()
            .map_err(|e| format!("git {}: {e}", args[0]))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git {}: {}", args[0], stderr.trim()));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }
}

impl Forge for CliForge {
    fn head(&self, worktree: &Path) -> Result<String, String> {
        self.git(worktree, &["rev-parse", "HEAD"])
    }

    fn remote_head(&self, worktree: &Path, branch: &str) -> Result<Option<String>, String> {
        let reference = format!("refs/heads/{branch}");
        let output = self
            .login("git", &["ls-remote", "origin", &reference], worktree)
            .output()
            .map_err(|e| format!("git ls-remote: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git ls-remote: {}", stderr.trim()));
        }
        let listed = String::from_utf8_lossy(&output.stdout);
        Ok(listed.split_whitespace().next().map(String::from))
    }

    fn push(
        &self,
        worktree: &Path,
        branch: &str,
        lease: Option<&str>,
        env: &[(String, String)],
        timeout: Duration,
    ) -> Result<(), String> {
        let refspec = format!("HEAD:refs/heads/{branch}");
        let lease = lease.map(|commit| format!("--force-with-lease=refs/heads/{branch}:{commit}"));
        let mut args = vec!["push", "--set-upstream"];
        args.extend(lease.as_deref());
        args.extend(["origin", refspec.as_str()]);
        let spec = self
            .login("git", &args, worktree)
            .envs(env.iter().cloned())
            .terminal();
        let exit = run_headless(&spec, HEADLESS_SESSION, Some(timeout))?;
        match (exit.timed_out, exit.code) {
            (true, _) => Err(format!("git push timed out: {}", tail(&exit.output))),
            (false, Some(0)) => Ok(()),
            _ => Err(format!("git push failed: {}", tail(exit.output.trim()))),
        }
    }

    fn last_subject(&self, worktree: &Path) -> Result<String, String> {
        self.git(worktree, &["log", "-1", "--format=%s"])
    }

    fn dirty_paths(&self, worktree: &Path) -> Result<Vec<String>, String> {
        let status = self.git(worktree, &["status", "--porcelain"])?;
        Ok(status
            .lines()
            .filter_map(|line| line.get(3..).map(String::from))
            .collect())
    }

    fn open_pull_request(
        &self,
        worktree: &Path,
        branch: &str,
    ) -> Result<Option<PullRequest>, String> {
        let args = [
            "pr", "list", "--head", branch, "--state", "open", "--limit", "1", "--json", PR_FIELDS,
        ];
        let listed: Value = serde_json::from_str(&self.gh(worktree, &args)?)
            .map_err(|e| format!("gh pr list: {e}"))?;
        listed
            .as_array()
            .and_then(|prs| prs.first())
            .map(pull_request)
            .transpose()
    }

    fn create_pull_request(
        &self,
        worktree: &Path,
        new: &NewPullRequest,
    ) -> Result<PullRequest, String> {
        let mut args = vec![
            "pr",
            "create",
            "--head",
            &new.branch,
            "--title",
            &new.title,
            "--body",
            &new.body,
            "--assignee",
            &new.assignee,
        ];
        if new.draft {
            args.push("--draft");
        }
        self.gh(worktree, &args)?;
        self.open_pull_request(worktree, &new.branch)?
            .ok_or_else(|| format!("gh pr create: no open PR for {} afterwards", new.branch))
    }

    fn edit_pull_request(
        &self,
        worktree: &Path,
        number: u64,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<(), String> {
        let number = number.to_string();
        let mut args = vec!["pr", "edit", number.as_str()];
        if let Some(title) = title {
            args.extend(["--title", title]);
        }
        if let Some(body) = body {
            args.extend(["--body", body]);
        }
        self.gh(worktree, &args).map(|_| ())
    }

    fn set_draft(&self, worktree: &Path, number: u64, draft: bool) -> Result<(), String> {
        let number = number.to_string();
        let mut args = vec!["pr", "ready", number.as_str()];
        if draft {
            args.push("--undo");
        }
        self.gh(worktree, &args).map(|_| ())
    }

    fn has_comment(&self, worktree: &Path, number: u64, marker: &str) -> Result<bool, String> {
        let number = number.to_string();
        let args = ["pr", "view", number.as_str(), "--json", "comments"];
        let viewed: Value = serde_json::from_str(&self.gh(worktree, &args)?)
            .map_err(|e| format!("gh pr view: {e}"))?;
        let comments = viewed["comments"].as_array().into_iter().flatten();
        Ok(comments
            .filter_map(|comment| comment["body"].as_str())
            .any(|body| body.contains(marker)))
    }

    fn comment(&self, worktree: &Path, number: u64, body: &str) -> Result<(), String> {
        let number = number.to_string();
        let args = ["pr", "comment", number.as_str(), "--body", body];
        self.gh(worktree, &args).map(|_| ())
    }
}

impl OpenPullRequests for CliForge {
    fn open(&self, urls: &[String]) -> Result<Vec<OpenPullRequest>, String> {
        let mut open = Vec::new();
        for url in urls {
            let args = ["pr", "view", url.as_str(), "--json", "state,files"];
            let viewed: Value = serde_json::from_str(&self.gh(&std::env::temp_dir(), &args)?)
                .map_err(|e| format!("gh pr view: {e}"))?;
            if viewed["state"] != "OPEN" {
                continue;
            }
            let files = viewed["files"].as_array().into_iter().flatten();
            open.push(OpenPullRequest {
                url: url.clone(),
                files: files
                    .filter_map(|file| file["path"].as_str().map(String::from))
                    .collect(),
            });
        }
        Ok(open)
    }
}

fn pull_request(value: &Value) -> Result<PullRequest, String> {
    let field = |key: &str| {
        value[key]
            .as_str()
            .map(String::from)
            .ok_or_else(|| format!("gh answered a PR without {key}"))
    };
    Ok(PullRequest {
        number: value["number"]
            .as_u64()
            .ok_or("gh answered a PR without number")?,
        url: field("url")?,
        title: field("title")?,
        body: value["body"].as_str().unwrap_or_default().to_string(),
        draft: value["isDraft"] == true,
    })
}

pub(super) fn tail(output: &str) -> &str {
    let mut start = output.len().saturating_sub(OUTPUT_TAIL);
    while !output.is_char_boundary(start) {
        start += 1;
    }
    &output[start..]
}
