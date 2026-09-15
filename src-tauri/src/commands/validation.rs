use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use super::git::{git_command, git_worktree_list};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    name: String,
    local_path: String,
    worktree_base_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    local_path: String,
    path: String,
    branch_name: String,
}

fn resolved_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err("Expected an absolute path without traversal".into());
    }
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|_| "Cannot resolve path".into());
    }
    let parent = path.parent().ok_or("Invalid path")?;
    Ok(resolved_path(parent)?.join(path.file_name().ok_or("Invalid filename")?))
}

fn repository_root(path: &str) -> Result<PathBuf, String> {
    let output = git_command()
        .args(["-C", path, "rev-parse", "--show-toplevel"])
        .output()
        .map_err(|_| "Cannot run Git")?;
    if !output.status.success() {
        return Err("Not a Git repository".into());
    }
    let root = resolved_path(Path::new(String::from_utf8_lossy(&output.stdout).trim()))?;
    if root != resolved_path(Path::new(path))? {
        return Err("Select the repository root".into());
    }
    Ok(root)
}

#[tauri::command]
pub async fn validate_workspace_repos(repos: Vec<Repository>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || validate_repos(&repos))
        .await
        .map_err(|_| "Validation failed")?
}

fn validate_repos(repos: &[Repository]) -> Result<(), String> {
    if repos.is_empty() {
        return Err("No repositories".into());
    }
    let mut roots = Vec::new();
    let mut bases: Vec<PathBuf> = Vec::new();
    for repo in repos {
        if repo.name.trim().is_empty() {
            return Err("Repository name is required".into());
        }
        let root = repository_root(&repo.local_path)?;
        let base = resolved_path(Path::new(&repo.worktree_base_path))?;
        if roots.contains(&root)
            || bases
                .iter()
                .any(|other| base.starts_with(other) || other.starts_with(&base))
        {
            return Err("Duplicate repositories or overlapping worktree directories".into());
        }
        if base == root {
            return Err("Worktree directory cannot be the repository root".into());
        }
        roots.push(root);
        bases.push(base);
    }
    Ok(())
}

#[tauri::command]
pub async fn validate_task_worktrees(
    members: Vec<Member>,
    base_paths: Vec<String>,
    owned_paths: Vec<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        validate_ownership(&members, &owned_paths)?;
        validate_members(&members, &base_paths)
    })
    .await
    .map_err(|_| "Validation failed")?
}

fn validate_ownership(members: &[Member], owned_paths: &[String]) -> Result<(), String> {
    let owned: Vec<_> = owned_paths
        .iter()
        .filter_map(|path| resolved_path(Path::new(path)).ok())
        .collect();
    for member in members {
        if owned.contains(&resolved_path(Path::new(&member.path))?) {
            return Err("A task already owns this worktree".into());
        }
    }
    Ok(())
}

fn validate_members(members: &[Member], base_paths: &[String]) -> Result<(), String> {
    if members.is_empty() || members.len() != base_paths.len() {
        return Err("Invalid members".into());
    }
    let mut destinations = Vec::new();
    for (member, base) in members.iter().zip(base_paths) {
        if member.branch_name.starts_with('-') || member.branch_name.starts_with('/') {
            return Err("Invalid branch".into());
        }
        let output = git_command()
            .args(["check-ref-format", "--branch", &member.branch_name])
            .output()
            .map_err(|_| "Cannot validate branch")?;
        if !output.status.success() {
            return Err("Invalid branch".into());
        }
        let root = repository_root(&member.local_path)?;
        let destination = resolved_path(Path::new(&member.path))?;
        let base = resolved_path(Path::new(base))?;
        if destination == root
            || destination == base
            || !destination.starts_with(&base)
            || destinations.contains(&destination)
        {
            return Err("Invalid destination".into());
        }
        let worktrees = git_worktree_list(member.local_path.clone())?;
        if Path::new(&member.path).exists()
            && !worktrees.iter().any(|tree| {
                resolved_path(Path::new(&tree.path)).ok().as_ref() == Some(&destination)
                    && tree.branch == member.branch_name
                    && !tree.bare
            })
        {
            return Err("Destination already exists and does not match this worktree".into());
        }
        if worktrees.iter().any(|tree| {
            tree.branch == member.branch_name
                && resolved_path(Path::new(&tree.path)).ok().as_ref() != Some(&destination)
        }) {
            return Err("Branch is already checked out elsewhere".into());
        }
        destinations.push(destination);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    struct Repo(PathBuf);
    impl Repo {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "wtm-validation-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            let repo = Self(path);
            repo.git(&["init", "--initial-branch=main"]);
            repo.git(&[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "fixture",
            ]);
            repo
        }
        fn git(&self, args: &[&str]) {
            let output = git_command()
                .arg("-C")
                .arg(&self.0)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        fn repository(&self) -> Repository {
            Repository {
                name: "test".into(),
                local_path: self.0.to_string_lossy().into_owned(),
                worktree_base_path: self.0.join("worktrees").to_string_lossy().into_owned(),
            }
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn validates_roots_and_rejects_duplicates_and_relative_paths() {
        let repo = Repo::new();
        assert!(validate_repos(&[repo.repository()]).is_ok());
        assert!(validate_repos(&[repo.repository(), repo.repository()]).is_err());
        let mut invalid = repo.repository();
        invalid.worktree_base_path = "relative/path".into();
        assert!(validate_repos(&[invalid]).is_err());
    }

    #[test]
    fn rejects_path_traversal_invalid_branches_and_mismatched_existing_worktrees() {
        let repo = Repo::new();
        let base = repo.repository().worktree_base_path;
        let mut member = Member {
            local_path: repo.repository().local_path,
            path: format!("{base}/feature"),
            branch_name: "feature".into(),
        };
        assert!(validate_members(
            &[Member {
                local_path: member.local_path.clone(),
                path: member.path.clone(),
                branch_name: member.branch_name.clone()
            }],
            std::slice::from_ref(&base)
        )
        .is_ok());
        member.branch_name = "--bad".into();
        assert!(
            validate_members(std::slice::from_ref(&member), std::slice::from_ref(&base)).is_err()
        );
        member.branch_name = "feature".into();
        member.path = format!("{base}/../escape");
        assert!(
            validate_members(std::slice::from_ref(&member), std::slice::from_ref(&base)).is_err()
        );
        member.path = format!("{base}/feature");
        repo.git(&["worktree", "add", "-b", "other", &member.path]);
        assert!(
            validate_members(std::slice::from_ref(&member), std::slice::from_ref(&base)).is_err()
        );
        member.branch_name = "other".into();
        assert!(
            validate_members(std::slice::from_ref(&member), std::slice::from_ref(&base)).is_ok()
        );
    }

    #[test]
    fn rejects_symlink_destinations_that_escape_the_configured_base() {
        let repo = Repo::new();
        let base = repo.0.join("worktrees");
        std::fs::create_dir(&base).unwrap();
        std::os::unix::fs::symlink(&repo.0, base.join("escape")).unwrap();
        let member = Member {
            local_path: repo.repository().local_path,
            path: base.join("escape/feature").to_string_lossy().into_owned(),
            branch_name: "feature".into(),
        };
        assert!(validate_members(&[member], &[base.to_string_lossy().into_owned()]).is_err());
    }

    #[test]
    fn detects_owned_paths_through_symlink_aliases() {
        let repo = Repo::new();
        let alias = repo.0.join("alias");
        std::os::unix::fs::symlink(&repo.0, &alias).unwrap();
        let member = Member {
            local_path: repo.repository().local_path,
            path: alias.join("feature").to_string_lossy().into_owned(),
            branch_name: "feature".into(),
        };
        let owned = repo.0.join("feature").to_string_lossy().into_owned();
        assert!(validate_ownership(&[member], &[owned]).is_err());
    }
}
