//! `pr_integrated`: the PR merged, or its changes are on the base branch although it was closed
//! without `mergedAt`, as a merge bot such as bors does.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{
    closed, conflict, staged, worktree, Github, Mergeability, Polls, Spawn, Target, WatchParams,
    PR_INTEGRATED,
};
use crate::commands::automations::wait::{Watch, WatchVerdict, Watcher};

const QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) { url number state mergedAt headRefOid baseRefName mergeable }
  }
  rateLimit { remaining resetAt }
}";

pub(super) struct PrIntegrated {
    github: Arc<Github>,
    polls: Arc<Polls<Integration>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequest {
    url: String,
    number: u64,
    state: String,
    merged_at: Option<String>,
    head_ref_oid: String,
    base_ref_name: String,
    mergeable: Mergeability,
}

pub(super) struct Integration {
    pr: PullRequest,
    /// For a PR closed without merging: whether its changes are on the base branch.
    on_base: Option<bool>,
}

impl PrIntegrated {
    pub(super) fn new(github: Arc<Github>, spawn: Spawn) -> Self {
        Self {
            github,
            polls: Polls::new(spawn),
        }
    }

    fn watch(&self, params: &WatchParams, watch: &Watch) -> Result<WatchVerdict, WatchVerdict> {
        let target = Target::of(watch, params)?;
        let (github, now_ms) = (self.github.clone(), watch.now_ms);
        let (fetch_target, worktree) = (target.clone(), worktree(watch, params.repo.as_deref()));
        let latest = self.polls.latest(
            &target.key(),
            now_ms,
            watch.armed_at_ms,
            params.interval_ms(),
            move || integration(&github, &fetch_target, worktree, now_ms),
        );
        Ok(latest.map_or(WatchVerdict::Pending, |latest| latest.verdict()))
    }
}

impl Watcher for PrIntegrated {
    fn check(&self, params: &Map<String, Value>, watch: &Watch) -> WatchVerdict {
        if watch.test {
            return staged(PR_INTEGRATED);
        }
        WatchParams::of(params)
            .and_then(|params| self.watch(&params, watch))
            .unwrap_or_else(|verdict| verdict)
    }
}

fn integration(
    github: &Github,
    target: &Target,
    worktree: Option<PathBuf>,
    now_ms: u64,
) -> Result<Integration, String> {
    let pr: PullRequest =
        github.pull_request(QUERY, &target.resolve(github.forge.as_ref())?, now_ms)?;
    let on_base = match (pr.state.as_str(), &pr.merged_at) {
        ("CLOSED", None) => {
            let worktree = worktree.ok_or("No worktree to check the base branch in")?;
            let forge = github.forge.as_ref();
            Some(forge.integrated(&worktree, pr.number, &pr.head_ref_oid, &pr.base_ref_name)?)
        }
        _ => None,
    };
    Ok(Integration { pr, on_base })
}

impl Integration {
    fn verdict(&self) -> WatchVerdict {
        let pr = &self.pr;
        let evidence = |verdict: &str| {
            json!({
                "verdict": verdict,
                "pr": pr.url,
                "head": pr.head_ref_oid,
                "base": pr.base_ref_name,
                "merged_at": pr.merged_at,
            })
        };
        match (pr.merged_at.is_some(), pr.state.as_str(), self.on_base) {
            (true, _, _) => WatchVerdict::Succeeded(Some(evidence("merged"))),
            (false, "CLOSED", Some(true)) => WatchVerdict::Succeeded(Some(evidence("integrated"))),
            (false, "CLOSED", _) => closed(
                evidence("pr_closed"),
                &format!("closed without its changes on {}", pr.base_ref_name),
            ),
            (false, "OPEN", _) if pr.mergeable == Mergeability::Conflicting => {
                conflict(evidence("conflict"), &pr.base_ref_name)
            }
            _ => WatchVerdict::Pending,
        }
    }
}
