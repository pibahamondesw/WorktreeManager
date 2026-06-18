use serde::Serialize;
use std::collections::HashMap;
use std::process::Command;

#[derive(Serialize)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub head: String,
    pub bare: bool,
}

/// Detect the default branch on the remote (main, master, etc.)
fn detect_default_branch(repo_path: &str) -> String {
    // Try reading the remote HEAD symbolic ref
    if let Ok(output) = Command::new("git")
        .args(["-C", repo_path, "symbolic-ref", "refs/remotes/origin/HEAD"])
        .output()
    {
        if output.status.success() {
            let refname = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // refs/remotes/origin/main -> main
            if let Some(branch) = refname.strip_prefix("refs/remotes/origin/") {
                return branch.to_string();
            }
        }
    }

    // Fallback: check if origin/main exists
    if let Ok(output) = Command::new("git")
        .args(["-C", repo_path, "rev-parse", "--verify", "origin/main"])
        .output()
    {
        if output.status.success() {
            return "main".to_string();
        }
    }

    // Fallback: check origin/master
    if let Ok(output) = Command::new("git")
        .args(["-C", repo_path, "rev-parse", "--verify", "origin/master"])
        .output()
    {
        if output.status.success() {
            return "master".to_string();
        }
    }

    "main".to_string()
}

#[tauri::command]
pub fn git_worktree_add(
    repo_path: String,
    worktree_path: String,
    branch_name: String,
) -> Result<String, String> {
    // Ensure the worktree parent directory exists
    let worktree = std::path::Path::new(&worktree_path);
    if let Some(parent) = worktree.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    // If the worktree already exists from a previous attempt, reuse it
    let wt_path = std::path::Path::new(&worktree_path);
    if wt_path.exists() && wt_path.join(".git").exists() {
        return Ok(format!("Worktree already exists at {}", worktree_path));
    }

    // 1. Fetch latest from origin so the new branch starts from up-to-date main
    let fetch = Command::new("git")
        .args(["-C", &repo_path, "fetch", "origin"])
        .output()
        .map_err(|e| format!("Failed to fetch from origin: {}", e))?;

    if !fetch.status.success() {
        let stderr = String::from_utf8_lossy(&fetch.stderr).to_string();
        return Err(format!("Failed to fetch from origin: {}", stderr));
    }

    // 2. Detect default branch (main/master/etc.)
    let default_branch = detect_default_branch(&repo_path);
    let start_point = format!("origin/{}", default_branch);

    // 3. Create worktree with new branch based on origin/<default>
    let output = Command::new("git")
        .args([
            "-C",
            &repo_path,
            "worktree",
            "add",
            &worktree_path,
            "-b",
            &branch_name,
            &start_point,
        ])
        .output()
        .map_err(|e| format!("Failed to execute git: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // If branch already exists, try checking it out into the worktree
        if stderr.contains("already exists") {
            let output2 = Command::new("git")
                .args([
                    "-C",
                    &repo_path,
                    "worktree",
                    "add",
                    &worktree_path,
                    &branch_name,
                ])
                .output()
                .map_err(|e| format!("Failed to execute git: {}", e))?;

            if output2.status.success() {
                Ok(String::from_utf8_lossy(&output2.stdout).to_string())
            } else {
                Err(String::from_utf8_lossy(&output2.stderr).to_string())
            }
        } else {
            Err(stderr)
        }
    }
}

/// Recursively copy a file or directory from `src` to `dst`.
fn copy_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

/// Copy gitignored local config (repo-relative `paths`) from the base repo into a
/// new worktree, skipping any path that's missing in the source or already present
/// in the worktree. Returns the paths actually copied.
#[tauri::command]
pub fn copy_local_configs(
    source_repo: String,
    worktree_path: String,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let src_root = std::path::Path::new(&source_repo);
    let dst_root = std::path::Path::new(&worktree_path);
    let mut copied = Vec::new();

    for rel in paths {
        // Guard against absolute paths and traversal escaping the repo root.
        if rel.is_empty() || std::path::Path::new(&rel).is_absolute() || rel.contains("..") {
            continue;
        }

        let src = src_root.join(&rel);
        let dst = dst_root.join(&rel);

        // Only copy what exists in the base repo and is missing in the worktree.
        if src.exists() && !dst.exists() {
            copy_recursive(&src, &dst).map_err(|e| format!("Failed to copy {}: {}", rel, e))?;
            copied.push(rel);
        }
    }

    Ok(copied)
}

#[tauri::command]
pub fn git_worktree_remove(repo_path: String, worktree_path: String) -> Result<String, String> {
    let wt_path = std::path::Path::new(&worktree_path);

    if !wt_path.exists() {
        // Directory already gone — prune stale git worktree references
        let _ = Command::new("git")
            .args(["-C", &repo_path, "worktree", "prune"])
            .output();
        return Ok("Worktree directory already removed".to_string());
    }

    let output = Command::new("git")
        .args([
            "-C",
            &repo_path,
            "worktree",
            "remove",
            &worktree_path,
            "--force",
        ])
        .output()
        .map_err(|e| format!("Failed to execute git: {}", e))?;

    if output.status.success() {
        Ok("Worktree removed successfully".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        // "not a working tree" means git no longer tracks it — prune and succeed
        if stderr.contains("not a working tree") {
            let _ = Command::new("git")
                .args(["-C", &repo_path, "worktree", "prune"])
                .output();
            Ok("Worktree reference cleaned up".to_string())
        } else {
            Err(stderr)
        }
    }
}

#[derive(Serialize, Clone)]
pub struct WorktreeStatus {
    pub ahead: i32,
    pub behind: i32,
    pub dirty: bool,
    pub last_commit_epoch: i64,
}

fn compute_worktree_status(worktree_path: &str, upstream: &str) -> WorktreeStatus {
    let (ahead, behind) = if let Ok(output) = Command::new("git")
        .args([
            "-C",
            worktree_path,
            "rev-list",
            "--left-right",
            "--count",
            &format!("HEAD...{}", upstream),
        ])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let parts: Vec<&str> = text.split('\t').collect();
            if parts.len() == 2 {
                (parts[0].parse().unwrap_or(0), parts[1].parse().unwrap_or(0))
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };

    let dirty = if let Ok(output) = Command::new("git")
        .args(["-C", worktree_path, "status", "--porcelain"])
        .output()
    {
        output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
    } else {
        false
    };

    let last_commit_epoch = if let Ok(output) = Command::new("git")
        .args(["-C", worktree_path, "log", "-1", "--format=%ct"])
        .output()
    {
        if output.status.success() {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse()
                .unwrap_or(0)
        } else {
            0
        }
    } else {
        0
    };

    WorktreeStatus {
        ahead,
        behind,
        dirty,
        last_commit_epoch,
    }
}

#[tauri::command]
pub fn git_worktree_status(
    worktree_path: String,
    repo_path: String,
) -> Result<WorktreeStatus, String> {
    let default_branch = detect_default_branch(&repo_path);
    let upstream = format!("origin/{}", default_branch);
    Ok(compute_worktree_status(&worktree_path, &upstream))
}

#[tauri::command]
pub async fn git_worktree_status_batch(
    worktree_paths: Vec<String>,
    repo_path: String,
) -> Result<HashMap<String, WorktreeStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let default_branch = detect_default_branch(&repo_path);
        let upstream = format!("origin/{}", default_branch);

        let mut result = HashMap::new();
        for path in &worktree_paths {
            result.insert(path.clone(), compute_worktree_status(path, &upstream));
        }
        result
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))
}

#[tauri::command]
pub fn git_remote_url(repo_path: String) -> Result<String, String> {
    let output = Command::new("git")
        .args(["-C", &repo_path, "remote", "get-url", "origin"])
        .output()
        .map_err(|e| format!("Failed to get remote URL: {}", e))?;

    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // Convert SSH URL to HTTPS: git@github.com:org/repo.git -> https://github.com/org/repo
        if url.starts_with("git@") {
            let cleaned = url.trim_end_matches(".git");
            let https = cleaned.replace(":", "/").replace("git@", "https://");
            Ok(https)
        } else {
            Ok(url.trim_end_matches(".git").to_string())
        }
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

#[tauri::command]
pub fn git_worktree_list(repo_path: String) -> Result<Vec<WorktreeInfo>, String> {
    let output = Command::new("git")
        .args(["-C", &repo_path, "worktree", "list", "--porcelain"])
        .output()
        .map_err(|e| format!("Failed to execute git: {}", e))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut worktrees = Vec::new();
    let mut current_path = String::new();
    let mut current_head = String::new();
    let mut current_branch = String::new();
    let mut is_bare = false;

    for line in stdout.lines() {
        if line.starts_with("worktree ") {
            if !current_path.is_empty() {
                worktrees.push(WorktreeInfo {
                    path: current_path.clone(),
                    branch: current_branch.clone(),
                    head: current_head.clone(),
                    bare: is_bare,
                });
            }
            current_path = line.strip_prefix("worktree ").unwrap_or("").to_string();
            current_head = String::new();
            current_branch = String::new();
            is_bare = false;
        } else if line.starts_with("HEAD ") {
            current_head = line.strip_prefix("HEAD ").unwrap_or("").to_string();
        } else if line.starts_with("branch ") {
            current_branch = line
                .strip_prefix("branch refs/heads/")
                .unwrap_or(line.strip_prefix("branch ").unwrap_or(""))
                .to_string();
        } else if line == "bare" {
            is_bare = true;
        }
    }

    // Push the last worktree
    if !current_path.is_empty() {
        worktrees.push(WorktreeInfo {
            path: current_path,
            branch: current_branch,
            head: current_head,
            bare: is_bare,
        });
    }

    Ok(worktrees)
}
