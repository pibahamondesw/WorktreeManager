use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::process::OwnedProcess;
use super::shell_env::{claude_env_prelude, login_shell, shell_single_quoted};

#[derive(serde::Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GithubPrStatus {
    state: String,
    is_draft: bool,
    ci: String,
    review: String,
}

fn validate_repo(slug: &str) -> Result<(&str, &str), String> {
    let parts: Vec<_> = slug.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || !p
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
    {
        return Err("Invalid GitHub repository".into());
    }
    Ok((parts[0], parts[1]))
}

fn run_process(
    command: &mut Command,
    input: &[u8],
    timeout: Duration,
    allow_partial: bool,
) -> Result<Vec<u8>, String> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut process = OwnedProcess::spawn(command)?;
    let mut stdout = process.child_mut().stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let mut stdin = process.child_mut().stdin.take().unwrap();
    let payload = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&payload));
    let deadline = Instant::now() + timeout;
    let mut completed = false;
    let result = loop {
        match process.poll() {
            Ok(Some(Some(0))) => {
                completed = true;
                break Ok(());
            }
            Ok(Some(_)) => {
                completed = true;
                break Err(
                    "GitHub command failed. Check authentication and repository access.".into(),
                );
            }
            Err(error) => break Err(error),
            Ok(None) if Instant::now() >= deadline => break Err("GitHub command timed out".into()),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    process.terminate();
    let _ = writer.join();
    let output = reader
        .join()
        .map_err(|_| "Could not read GitHub response")?
        .map_err(|_| "Could not read GitHub response")?;
    if completed
        && allow_partial
        && serde_json::from_slice::<Value>(&output)
            .ok()
            .is_some_and(|value| value["data"]["repository"].is_object())
    {
        return Ok(output);
    }
    result.map(|_| output)
}

fn run_gh(args: &[&str], input: &[u8]) -> Result<Vec<u8>, String> {
    let allow_partial = args.first() == Some(&"api");
    let args = args
        .iter()
        .map(|a| shell_single_quoted(a))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!("{} >/dev/null; exec gh {}", claude_env_prelude(), args);
    run_process(
        &mut login_shell(&script),
        input,
        Duration::from_secs(30),
        allow_partial,
    )
}

fn batch_payload(repo_slug: &str, numbers: &[u32]) -> Result<Value, String> {
    let (owner, name) = validate_repo(repo_slug)?;
    if numbers.contains(&0) {
        return Err("Invalid pull request number".into());
    }
    let fields = numbers.iter().collect::<BTreeSet<_>>().into_iter().map(|number| format!(
        "pr{number}: pullRequest(number: {number}) {{ state isDraft reviewDecision commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ state }} }} }} }} }}"
    )).collect::<Vec<_>>().join(" ");
    Ok(
        json!({"query": format!("query($owner: String!, $name: String!) {{ repository(owner: $owner, name: $name) {{ {fields} }} }}"), "variables": {"owner": owner, "name": name}}),
    )
}

fn parse_statuses(bytes: &[u8]) -> Result<HashMap<u32, GithubPrStatus>, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid GitHub response")?;
    let repo = value
        .pointer("/data/repository")
        .and_then(Value::as_object)
        .ok_or("GitHub repository unavailable")?;
    let mut statuses = HashMap::new();
    for (alias, pr) in repo {
        let Some(number) = alias.strip_prefix("pr").and_then(|n| n.parse().ok()) else {
            continue;
        };
        let state = match pr["state"].as_str() {
            Some("OPEN") => "open",
            Some("CLOSED") => "closed",
            Some("MERGED") => "merged",
            _ => continue,
        };
        let Some(is_draft) = pr["isDraft"].as_bool() else {
            continue;
        };
        let ci = match pr
            .pointer("/commits/nodes/0/commit/statusCheckRollup/state")
            .and_then(Value::as_str)
        {
            Some("SUCCESS") => "passing",
            Some("FAILURE" | "ERROR") => "failing",
            Some("PENDING" | "EXPECTED") => "running",
            None => "none",
            _ => "unknown",
        };
        let review = match pr["reviewDecision"].as_str() {
            Some("APPROVED") => "approved",
            Some("CHANGES_REQUESTED") => "changes_requested",
            _ => "pending",
        };
        statuses.insert(
            number,
            GithubPrStatus {
                state: state.into(),
                is_draft,
                ci: ci.into(),
                review: review.into(),
            },
        );
    }
    Ok(statuses)
}

#[tauri::command]
pub async fn github_pr_status_batch(
    repo_slug: String,
    pr_numbers: Vec<u32>,
) -> Result<HashMap<u32, GithubPrStatus>, String> {
    let payload = batch_payload(&repo_slug, &pr_numbers)?;
    if pr_numbers.is_empty() {
        return Ok(HashMap::new());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = run_gh(
            &["api", "graphql", "--hostname", "github.com", "--input", "-"],
            &serde_json::to_vec(&payload).unwrap(),
        )?;
        parse_statuses(&bytes)
    })
    .await
    .map_err(|_| "GitHub task failed".to_string())?
}

#[tauri::command]
pub async fn github_pr_ready(repo_slug: String, pr_number: u32) -> Result<(), String> {
    validate_repo(&repo_slug)?;
    if pr_number == 0 {
        return Err("Invalid pull request number".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        run_gh(
            &[
                "pr",
                "ready",
                &pr_number.to_string(),
                "--repo",
                &format!("github.com/{repo_slug}"),
            ],
            &[],
        )
        .map(|_| ())
    })
    .await
    .map_err(|_| "GitHub task failed".to_string())?
}

pub(super) fn authenticated() -> bool {
    run_gh(
        &["auth", "status", "--active", "--hostname", "github.com"],
        &[],
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_deduplicates_and_validates_inputs() {
        let payload = batch_payload("org/repo", &[2, 2, 3]).unwrap();
        assert_eq!(
            payload["query"].as_str().unwrap().matches("pr2:").count(),
            1
        );
        assert_eq!(payload["variables"]["owner"], "org");
        assert!(batch_payload("org/repo;bad", &[2]).is_err());
        assert!(batch_payload("org/repo", &[0]).is_err());
    }

    #[test]
    fn partial_response_preserves_valid_prs() {
        let bytes = br#"{"data":{"repository":{"pr1":{"state":"OPEN","isDraft":true,"reviewDecision":"APPROVED","commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"FAILURE"}}}]}},"pr2":null}},"errors":[{"message":"missing"}]}"#;
        let statuses = parse_statuses(bytes).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[&1].ci, "failing");
        assert_eq!(statuses[&1].review, "approved");
        assert!(statuses[&1].is_draft);
        assert!(parse_statuses(b"bad").is_err());
    }

    #[test]
    fn process_times_out_and_reports_failures() {
        let mut slow = Command::new("/bin/sh");
        slow.args(["-c", "sleep 10"]);
        assert_eq!(
            run_process(&mut slow, &[], Duration::from_millis(30), false).unwrap_err(),
            "GitHub command timed out"
        );
        let mut failed = Command::new("/bin/sh");
        failed.args(["-c", "exit 1"]);
        assert!(run_process(&mut failed, &[], Duration::from_secs(1), false).is_err());
    }
    #[test]
    fn process_preserves_graphql_data_on_nonzero_exit_and_passes_stdin() {
        let payload = br#"{"data":{"repository":{"pr1":null}},"errors":[{"message":"missing"}]}"#;
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "cat; exit 1"]);
        assert_eq!(
            run_process(&mut command, payload, Duration::from_secs(1), true).unwrap(),
            payload
        );
    }

    #[test]
    fn normalizes_all_ci_and_review_states() {
        for (raw, expected) in [
            ("SUCCESS", "passing"),
            ("FAILURE", "failing"),
            ("ERROR", "failing"),
            ("PENDING", "running"),
            ("EXPECTED", "running"),
            ("NEW", "unknown"),
        ] {
            let bytes = serde_json::to_vec(&json!({"data": {"repository": {"pr1": {
                "state": "OPEN", "isDraft": false, "reviewDecision": "REVIEW_REQUIRED",
                "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": raw}}}]}
            }}}}))
            .unwrap();
            let status = parse_statuses(&bytes).unwrap().remove(&1).unwrap();
            assert_eq!(status.ci, expected);
            assert_eq!(status.review, "pending");
        }
        for (raw, expected) in [
            ("APPROVED", "approved"),
            ("CHANGES_REQUESTED", "changes_requested"),
            ("REVIEW_REQUIRED", "pending"),
        ] {
            let bytes = serde_json::to_vec(&json!({"data": {"repository": {"pr1": {
                "state": "MERGED", "isDraft": false, "reviewDecision": raw,
                "commits": {"nodes": [{"commit": {"statusCheckRollup": null}}]}
            }}}}))
            .unwrap();
            let status = parse_statuses(&bytes).unwrap().remove(&1).unwrap();
            assert_eq!(status.state, "merged");
            assert_eq!(status.review, expected);
            assert_eq!(status.ci, "none");
        }
    }
}
