//! Branch names of the tasks automations create.

use sha2::{Digest, Sha256};

use super::TaskFailure;
use crate::commands::validation::is_valid_branch_name;

const ID_HASH_CHARS: usize = 8;

/// The item id as one component of a branch name, `{item.branch}`. An id git accepts as is stays
/// as is; any other becomes a ref-safe slug suffixed with a short hash of the id, so distinct ids
/// never share a branch. `/` is replaced too: `x` and `x/y` cannot both be branches.
pub(super) fn item_branch(id: &str) -> String {
    let slug = ref_safe_slug(id);
    if slug == id {
        return slug;
    }
    let digest = format!("{:x}", Sha256::digest(id.as_bytes()));
    let hash = &digest[..ID_HASH_CHARS];
    if slug.is_empty() {
        hash.to_string()
    } else {
        format!("{slug}-{hash}")
    }
}

fn ref_safe_slug(id: &str) -> String {
    let mapped: String = id
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '-',
        })
        .collect();
    let dotted = mapped
        .split('.')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(".");
    let slug = dotted.trim_start_matches('-');
    match slug.strip_suffix(".lock") {
        Some(stem) => format!("{stem}-lock"),
        None => slug.to_string(),
    }
}

/// A rendered branch git accepts; any other fails task creation naming it.
pub(super) fn checked_branch(branch: String) -> Result<String, TaskFailure> {
    if is_valid_branch_name(&branch).map_err(TaskFailure::new)? {
        return Ok(branch);
    }
    Err(TaskFailure::new(format!(
        "branch {branch:?} is not a valid git branch name"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_git_accepts_keep_their_branch() {
        for id in ["a", "issue-12", "v1.2_rc", "A_b-C"] {
            assert_eq!(item_branch(id), id);
        }
    }

    #[test]
    fn any_selector_id_renders_a_valid_and_distinct_branch() {
        let ids = [
            "close:crossborder",
            "close-crossborder",
            "a@{b",
            "a..b",
            "a.b",
            "a//b",
            "a/b",
            "a.",
            "a/",
            "a",
            ".a",
            "a.lock",
            "a-lock",
            "a/b.lock/c",
            "@",
            "-a",
            "...",
            ":",
        ];
        let branches: Vec<String> = ids.iter().map(|id| item_branch(id)).collect();
        for (id, branch) in ids.iter().zip(&branches) {
            assert!(
                is_valid_branch_name(branch).unwrap(),
                "{id:?} -> {branch:?}"
            );
            assert!(
                is_valid_branch_name(&format!("wtm/demo/{branch}")).unwrap(),
                "{id:?} -> {branch:?}"
            );
            assert!(!branch.contains('/'), "{id:?} -> {branch:?}");
        }
        let mut distinct = branches.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), ids.len(), "{branches:?}");
        assert_eq!(
            item_branch("close:crossborder"),
            format!(
                "close-crossborder-{}",
                &format!("{:x}", Sha256::digest(b"close:crossborder"))[..8]
            )
        );
    }

    #[test]
    fn checked_branch_names_a_branch_git_rejects() {
        assert_eq!(
            checked_branch("wtm/demo/a".into()).ok().as_deref(),
            Some("wtm/demo/a")
        );
        let failure = checked_branch("wtm/demo/a:b".into()).unwrap_err();
        assert_eq!(
            failure.message,
            "branch \"wtm/demo/a:b\" is not a valid git branch name"
        );
    }
}
