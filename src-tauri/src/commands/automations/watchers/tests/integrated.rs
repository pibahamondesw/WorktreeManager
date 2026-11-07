use super::*;

fn closed(merged_at: Option<&str>) -> Value {
    let mut pr = open_pr(HEAD, Vec::new());
    pr["state"] = json!(if merged_at.is_some() {
        "MERGED"
    } else {
        "CLOSED"
    });
    pr["mergedAt"] = json!(merged_at);
    pr
}

fn evidence(verdict: &str, merged_at: Option<&str>) -> Option<Value> {
    Some(
        json!({ "verdict": verdict, "pr": PR_URL, "head": HEAD, "base": "main",
        "merged_at": merged_at }),
    )
}

fn closed_evidence() -> Option<Value> {
    let mut evidence = evidence("pr_closed", None).unwrap();
    evidence["summary"] = json!(format!(
        "The PR {PR_URL} was closed without its changes on main"
    ));
    Some(evidence)
}

fn integrated(waiting: &Waiting, forge: &Arc<FakeForge>) -> WatchVerdict {
    waiting.at(
        &inline_registry(forge),
        PR_INTEGRATED,
        json!({ "repo": "app" }),
        NOW,
    )
}

#[test]
fn an_open_pr_keeps_waiting_and_a_merged_one_is_integrated() {
    let forge = FakeForge::answering(open_pr(HEAD, Vec::new()));
    let waiting = Waiting::published();

    assert_eq!(integrated(&waiting, &forge), WatchVerdict::Pending);
    let merged_at = Some("2026-11-05T13:00:00Z");
    forge.set(closed(merged_at));
    assert_eq!(
        integrated(&waiting, &forge),
        WatchVerdict::Succeeded(evidence("merged", merged_at))
    );
    assert!(forge.integration_checks.lock().unwrap().is_empty());
}

#[test]
fn an_open_pr_that_conflicts_with_its_base_fails_and_one_not_yet_known_keeps_waiting() {
    let forge = FakeForge::answering(mergeable(open_pr(HEAD, Vec::new()), "CONFLICTING"));
    let waiting = Waiting::published();

    let mut conflict = evidence("conflict", None).unwrap();
    conflict["conflict"] =
        json!("The PR conflicts with main: rebase it onto origin/main and resolve the conflicts");
    assert_eq!(
        integrated(&waiting, &forge),
        WatchVerdict::Failed("conflict".into(), Some(conflict))
    );
    forge.set(mergeable(open_pr(HEAD, Vec::new()), "UNKNOWN"));
    assert_eq!(integrated(&waiting, &forge), WatchVerdict::Pending);
    forge.set(mergeable(
        closed(Some("2026-11-05T13:00:00Z")),
        "CONFLICTING",
    ));
    assert_eq!(
        integrated(&waiting, &forge),
        WatchVerdict::Succeeded(evidence("merged", Some("2026-11-05T13:00:00Z")))
    );
}

#[test]
fn a_pr_closed_without_merged_at_is_integrated_when_its_changes_are_on_the_base() {
    let forge = FakeForge::answering(closed(None));
    let waiting = Waiting::published();

    *forge.on_base.lock().unwrap() = Some(Ok(true));
    assert_eq!(
        integrated(&waiting, &forge),
        WatchVerdict::Succeeded(evidence("integrated", None))
    );
    assert_eq!(
        forge.integration_checks.lock().unwrap()[0],
        (
            PathBuf::from("/w/app"),
            7,
            HEAD.to_string(),
            "main".to_string()
        )
    );

    *forge.on_base.lock().unwrap() = Some(Ok(false));
    assert_eq!(
        integrated(&waiting, &forge),
        WatchVerdict::Blocked("pr_closed".into(), closed_evidence())
    );

    *forge.on_base.lock().unwrap() = Some(Err("Could not fetch main".into()));
    assert_eq!(integrated(&waiting, &forge), WatchVerdict::Pending);

    let without_task = Waiting {
        task: None,
        ..Waiting::published()
    };
    assert_eq!(integrated(&without_task, &forge), WatchVerdict::Pending);
    assert_eq!(forge.integration_checks.lock().unwrap().len(), 3);
    assert_eq!(
        without_task.at(&inline_registry(&forge), PR_INTEGRATED, json!({ "x": 1 }), NOW),
        WatchVerdict::Failed("invalid_params: unknown field `x`, expected one of `required`, `ignore`, `bot_lgtm`, `unresolved_threads`, `request_review_comment`, `interval_secs`, `repo`".into(), None)
    );
    let unpublished = Waiting {
        history: Vec::new(),
        task: None,
        ..Waiting::published()
    };
    assert_eq!(
        integrated(&unpublished, &forge),
        WatchVerdict::Failed("no_task".into(), None)
    );
}
