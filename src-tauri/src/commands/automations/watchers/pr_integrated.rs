//! `pr_integrated`: the PR merged, or its changes are on the base branch although it was closed
//! without `mergedAt`, as a merge bot such as bors does.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{worktree, Github, Polls, Spawn, Target, WatchParams};
use crate::commands::automations::wait::{Watch, WatchVerdict, Watcher};

const QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) { url number state mergedAt headRefOid baseRefName }
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
        let (fetch_target, worktree) = (target.clone(), worktree(watch, params));
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
            Some(json!({
                "verdict": verdict,
                "pr": pr.url,
                "head": pr.head_ref_oid,
                "base": pr.base_ref_name,
                "merged_at": pr.merged_at,
            }))
        };
        match (pr.merged_at.is_some(), pr.state.as_str(), self.on_base) {
            (true, _, _) => WatchVerdict::Succeeded(evidence("merged")),
            (false, "CLOSED", Some(true)) => WatchVerdict::Succeeded(evidence("integrated")),
            (false, "CLOSED", _) => WatchVerdict::Failed("pr_closed".into(), evidence("pr_closed")),
            _ => WatchVerdict::Pending,
        }
    }
}
