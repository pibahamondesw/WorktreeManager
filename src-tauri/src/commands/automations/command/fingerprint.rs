use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::commands::git::git_command;

const UNSTAGED_DIFF: &[&str] = &["diff", "--binary", "--no-ext-diff", "--no-textconv"];
const UNTRACKED_FILES: &[&str] = &["ls-files", "--others", "--exclude-standard", "-z"];

/// The state of a worktree a receipt vouches for. Any commit, branch switch, staged change,
/// edit of a tracked file or untracked file changes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFingerprint {
    pub head: Option<String>,
    pub branch: Option<String>,
    /// Digest of the staged entries.
    pub index: String,
    /// Digest of unstaged changes and of untracked files with their contents.
    pub worktree: String,
}

impl GitFingerprint {
    pub fn of(worktree: &Path) -> Result<Self, String> {
        let index = git(worktree, &["ls-files", "--stage", "-z"])?;
        let unstaged = git(worktree, UNSTAGED_DIFF)?;
        let untracked = git(worktree, UNTRACKED_FILES)?;
        let mut changes = Sha256::new();
        changes.update(&unstaged);
        for path in untracked.split(|&byte| byte == 0).filter(|p| !p.is_empty()) {
            changes.update(path);
            changes.update(content_digest(&worktree.join(OsStr::from_bytes(path))));
        }
        Ok(Self {
            head: git_line(worktree, &["rev-parse", "--verify", "--quiet", "HEAD"]),
            branch: git_line(worktree, &["symbolic-ref", "--quiet", "--short", "HEAD"]),
            index: format!("{:x}", Sha256::digest(&index)),
            worktree: format!("{:x}", changes.finalize()),
        })
    }
}

fn content_digest(path: &Path) -> String {
    fs::read(path).map_or_else(
        |_| "unreadable".into(),
        |bytes| format!("{:x}", Sha256::digest(bytes)),
    )
}

fn git(worktree: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = git_command()
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(worktree)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args[0]))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

/// `None` for an unborn HEAD or a detached one.
fn git_line(worktree: &Path, args: &[&str]) -> Option<String> {
    let line = String::from_utf8(git(worktree, args).ok()?).ok()?;
    Some(line.trim().to_string()).filter(|line| !line.is_empty())
}
