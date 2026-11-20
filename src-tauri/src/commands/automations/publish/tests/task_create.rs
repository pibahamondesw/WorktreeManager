use super::*;
use crate::commands::automations::outputs::attempt_result;

fn workspaces() -> Value {
    json!([
        { "id": "w0", "name": "Other", "repos": [] },
        { "id": "w1", "name": "Acme", "repos": [
            { "id": "r1", "name": "api", "localPath": "/repos/api" },
            { "id": "r2", "name": "web", "localPath": "/repos/web" },
        ] },
    ])
}

fn listed_task(id: &str, branch: &str, repos: &[&str], issue: Option<&str>) -> Value {
    let members: Vec<Value> = repos
        .iter()
        .map(|repo| json!({ "repoId": repo, "path": format!("/wt/{repo}/{branch}") }))
        .collect();
    let mut task = json!({ "id": id, "branchName": branch, "members": members });
    if let Some(issue) = issue {
        task["linearIssueId"] = json!(format!("id-{issue}"));
        task["linearIssueIdentifier"] = json!(issue);
    }
    task
}

/// Answers the workspaces, the listed tasks and creates or links tasks on the asked branch.
fn tasks_bridge(listed: Vec<Value>) -> FakeBridge {
    FakeBridge::answering(move |method, params| match method {
        "workspace.list" => ok(workspaces()),
        "task.list" => ok(json!(listed)),
        "task.create" => {
            let input = &params["input"];
            let repos: Vec<&str> = input["repoIds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|repo| repo.as_str().unwrap())
                .collect();
            let branch = input["branchName"].as_str().unwrap();
            json!({
                "ok": true,
                "data": listed_task("t2", branch, &repos, None),
                "warnings": [{ "stage": "open", "message": "Could not open the editor" }],
            })
        }
        "task.link-issue" => ok(listed_task(
            params["id"].as_str().unwrap(),
            "feature/review",
            &["r1", "r2"],
            params["issue"].as_str(),
        )),
        "task.log-append" => ok(json!({ "status": "appended" })),
        "task.chat" => ok(json!({ "conversationId": "c1" })),
        "linear.issue" => ok(json!({
            "id": "id-ENG-7",
            "identifier": params["issue"],
            "branchName": "me/eng-7-review",
        })),
        _ => panic!("unexpected call to {method}"),
    })
}

fn review() -> TaskCreateEffect {
    TaskCreateEffect {
        workspace: Some("{steps.triage.result.workspace}".into()),
        repos: vec!["{steps.triage.result.repos}".into()],
        branch: Some("{steps.triage.result.branch}".into()),
        from_remote: vec!["{steps.triage.result.repo}".into()],
        linear_issue: Some("{steps.triage.result.issue}".into()),
    }
}

fn triaged(issue: &str) -> Run {
    let mut run = Run::all();
    run.variables = [
        ("workspace", "Acme"),
        ("repos", "[\"web\", \"api\"]"),
        ("branch", " feature/review "),
        ("repo", "api"),
        ("issue", issue),
    ]
    .into_iter()
    .map(|(key, value)| (format!("steps.triage.result.{key}"), value.to_string()))
    .collect();
    run
}

fn methods(bridge: &FakeBridge) -> Vec<String> {
    bridge
        .calls()
        .into_iter()
        .map(|(method, _)| method)
        .collect()
}

fn call<'a>(bridge: &'a [(String, Value)], method: &str) -> &'a Value {
    &bridge
        .iter()
        .find(|(called, _)| called == method)
        .unwrap()
        .1
}

fn output(run: &Run) -> Value {
    attempt_result(run.recorded.borrow().iter()).unwrap()
}

#[test]
fn without_params_it_reuses_the_task_on_the_item_branch_and_works_in_it() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(tasks_bridge(vec![
        listed_task("t0", "other", &["r1"], None),
        listed_task("t1", "wtm/demo/1", &["r1"], None),
    ]));
    let run = Run::all();
    let mut publisher = publisher(&forge, &bridge);

    let (report, effects) = run.publish(
        &mut publisher,
        1,
        vec![Effect::TaskCreate(TaskCreateEffect::default())],
    );

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(effects[0].status, EffectStatus::Reconciled);
    assert_eq!(
        effects[0].detail.as_deref(),
        Some("A task is on wtm/demo/1")
    );
    assert_eq!(effects[0].reference.as_deref(), Some("t1"));
    assert_eq!(
        output(&run)["wtm.task_create"],
        json!({
            "status": "reconciled",
            "reference": "t1",
            "branch": "wtm/demo/1",
            "worktrees": { "api": "/wt/r1/wtm/demo/1" },
        })
    );
    assert_eq!(methods(&bridge), ["workspace.list", "task.list"]);
    let (task, created) = publisher.take_task().unwrap();
    assert_eq!((task.id.as_str(), created), ("t1", false));
    assert!(publisher.take_task().is_none());
}

#[test]
fn without_params_it_creates_the_task_on_the_exact_branch_with_the_bases() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(tasks_bridge(Vec::new()));
    let mut run = Run::all();
    run.bases = BTreeMap::from([("r1".into(), "origin/develop".into())]);
    let mut publisher = publisher(&forge, &bridge);

    let (report, effects) = run.publish(
        &mut publisher,
        1,
        vec![Effect::TaskCreate(TaskCreateEffect::default())],
    );

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(effects[0].status, EffectStatus::Executed);
    assert_eq!(effects[0].reference.as_deref(), Some("t2"));
    assert_eq!(effects[0].warnings, ["Could not open the editor"]);
    assert_eq!(
        call(&bridge.calls(), "task.create"),
        &json!({ "input": {
            "workspaceId": "w1",
            "branchName": "wtm/demo/1",
            "repoIds": ["r1"],
            "automationId": "demo",
            "exactBranch": true,
            "baseRefs": { "r1": "origin/develop" },
        } })
    );
    assert!(publisher.take_task().unwrap().1);
    assert!(forge.calls().is_empty());

    let unbased = Arc::new(tasks_bridge(Vec::new()));
    Run::all().publish(
        &mut super::publisher(&forge, &unbased),
        1,
        vec![Effect::TaskCreate(TaskCreateEffect::default())],
    );
    assert!(call(&unbased.calls(), "task.create")["input"]
        .get("baseRefs")
        .is_none());
}

#[test]
fn a_review_task_checks_out_the_remote_head_links_its_issue_and_takes_the_next_effects() {
    let mut forge = FakeForge::at("abc");
    forge.remote_head = Some("abc".into());
    let forge = Arc::new(forge);
    let bridge = Arc::new(tasks_bridge(Vec::new()));
    let run = triaged("ENG-7");
    let log = Effect::TaskLog {
        entry: "Reviewing in {task.id} on {task.branch}".into(),
    };
    let chat = Effect::ChatOpen(chat_open("Review"));
    let mut publisher = publisher(&forge, &bridge);

    let (report, effects) = run.publish(
        &mut publisher,
        1,
        vec![Effect::TaskCreate(review()), log, chat],
    );

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(
        effects[0].detail.as_deref(),
        Some("Created a task on feature/review; linked it to ENG-7")
    );
    let calls = bridge.calls();
    assert_eq!(
        call(&calls, "task.create"),
        &json!({ "input": {
            "workspaceId": "w1",
            "branchName": "feature/review",
            "repoIds": ["r2", "r1"],
            "automationId": "demo",
            "exactBranch": true,
            "baseRefs": { "r1": "origin/feature/review" },
        } })
    );
    assert_eq!(
        call(&calls, "task.link-issue"),
        &json!({ "id": "t2", "issue": "ENG-7" })
    );
    assert_eq!(
        call(&calls, "task.log-append")["entry"],
        "Reviewing in t2 on feature/review"
    );
    assert_eq!(call(&calls, "task.chat")["id"], "t2");
    assert_eq!(
        forge.calls(),
        ["remote_head feature/review", "remote_head feature/review"]
    );
}

#[test]
fn a_reused_task_follows_the_remote_head_and_keeps_its_issue() {
    let behind = |error: Option<&str>| {
        let mut forge = FakeForge::at("abc");
        forge.remote_head = Some("def".into());
        forge.fast_forward_error = error.map(String::from);
        Arc::new(forge)
    };
    let existing = listed_task("t1", "feature/review", &["r1"], Some("ENG-1"));

    let forge = behind(None);
    let bridge = Arc::new(tasks_bridge(vec![existing.clone()]));
    let run = triaged("ENG-7");
    let (_, effects) = run.publish(
        &mut publisher(&forge, &bridge),
        2,
        vec![Effect::TaskCreate(review())],
    );
    assert_eq!(effects[0].status, EffectStatus::Executed);
    assert_eq!(
        effects[0].detail.as_deref(),
        Some("fast-forwarded api to origin/feature/review")
    );
    assert_eq!(
        effects[0].warnings,
        [
            "The task keeps the Linear issue ENG-1 instead of ENG-7",
            "The task has no worktree for web",
        ]
    );
    assert_eq!(
        forge.calls(),
        [
            "remote_head feature/review",
            "fast_forward /wt/r1/feature/review feature/review"
        ]
    );
    assert!(!methods(&bridge).contains(&"task.create".to_string()));

    let forge = behind(Some("Not possible to fast-forward"));
    let bridge = Arc::new(tasks_bridge(vec![existing.clone()]));
    let (report, effects) = triaged("ENG-1").publish(
        &mut publisher(&forge, &bridge),
        2,
        vec![Effect::TaskCreate(review())],
    );
    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(effects[0].status, EffectStatus::Reconciled);
    assert_eq!(
        effects[0].warnings,
        [
            "The task has no worktree for web",
            "api stays where it is: Not possible to fast-forward",
        ]
    );

    let gone = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(tasks_bridge(vec![existing]));
    let (_, effects) = triaged("").publish(
        &mut publisher(&gone, &bridge),
        2,
        vec![Effect::TaskCreate(review())],
    );
    assert_eq!(
        effects[0].warnings[1],
        "api stays where it is: origin has no branch feature/review"
    );
}

#[test]
fn a_reused_task_without_an_issue_is_linked_to_it() {
    let mut forge = FakeForge::at("abc");
    forge.remote_head = Some("abc".into());
    let forge = Arc::new(forge);
    let bridge = Arc::new(tasks_bridge(vec![listed_task(
        "t1",
        "feature/review",
        &["r1", "r2"],
        None,
    )]));

    let (_, effects) = triaged("ENG-7").publish(
        &mut publisher(&forge, &bridge),
        1,
        vec![Effect::TaskCreate(review())],
    );

    assert_eq!(effects[0].detail.as_deref(), Some("linked it to ENG-7"));
    assert!(effects[0].warnings.is_empty());
}

#[test]
fn a_branch_missing_on_origin_fails_before_creating_anything() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(tasks_bridge(Vec::new()));

    let (report, effects) = triaged("ENG-7").publish(
        &mut publisher(&forge, &bridge),
        1,
        vec![Effect::TaskCreate(review())],
    );

    assert_eq!(report.outcome, StepOutcome::Failure);
    assert_eq!(
        report.cause.as_deref(),
        Some("origin of api has no branch feature/review")
    );
    assert_eq!(effects[0].status, EffectStatus::Failed);
    assert_eq!(methods(&bridge), ["workspace.list", "task.list"]);
}

#[test]
fn a_partial_creation_names_the_worktrees_it_left_and_does_not_move_the_item() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(FakeBridge::answering(|method, _| match method {
        "workspace.list" => ok(workspaces()),
        "task.list" => ok(json!([])),
        _ => json!({ "ok": false, "error": {
            "code": "create_failed",
            "message": "Some worktrees could not be created.",
            "details": { "completedMembers": [{ "repoId": "r1", "path": "/wt/api/x" }] },
        } }),
    }));
    let mut publisher = publisher(&forge, &bridge);

    let (report, _) = Run::all().publish(
        &mut publisher,
        1,
        vec![Effect::TaskCreate(TaskCreateEffect::default())],
    );

    assert_eq!(
        report.cause.as_deref(),
        Some("Some worktrees could not be created.; worktrees left on disk: /wt/api/x")
    );
    assert!(publisher.take_task().is_none());
}

#[test]
fn what_the_run_decides_must_name_a_workspace_repositories_and_a_valid_branch() {
    let forge = Arc::new(FakeForge::at("abc"));
    let cases = [
        ("workspace", "Missing", "There is no workspace Missing"),
        (
            "repos",
            "[\"api\", \"ios\"]",
            "Workspace Acme has no repository ios",
        ),
        ("repos", "[]", "repos rendered no repository"),
        ("repo", "ios", "Workspace Acme has no repository ios"),
        (
            "branch",
            "feature..review",
            "branch \"feature..review\" is not a valid git branch name",
        ),
    ];
    for (key, value, cause) in cases {
        let bridge = Arc::new(tasks_bridge(Vec::new()));
        let mut run = triaged("ENG-7");
        let name = format!("steps.triage.result.{key}");
        run.variables.retain(|(variable, _)| *variable != name);
        run.variables.push((name, value.into()));
        let effect = TaskCreateEffect {
            from_remote: if key == "repos" {
                Vec::new()
            } else {
                review().from_remote
            },
            ..review()
        };

        let (report, _) = run.publish(
            &mut publisher(&forge, &bridge),
            1,
            vec![Effect::TaskCreate(effect)],
        );

        assert_eq!(report.cause.as_deref(), Some(cause), "{key}={value}");
    }
}

/// An answer that replaces the usual one for its method.
type Override = fn(&str) -> Option<Value>;

#[test]
fn operation_failures_fail_the_effect() {
    let forge = Arc::new(FakeForge::at("abc"));
    let answering = |answer: Override| {
        Arc::new(FakeBridge::answering(move |method, params| {
            answer(method).unwrap_or_else(|| (tasks_bridge(Vec::new()).answer)(method, params))
        }))
    };
    let cases: [(Override, &str); 4] = [
        (
            |method| (method == "workspace.list").then(|| failure("offline")),
            "offline happened",
        ),
        (
            |method| (method == "task.list").then(|| failure("not_found")),
            "not_found happened",
        ),
        (
            |method| {
                (method == "task.list")
                    .then(|| ok(json!([{ "id": "t1", "branchName": "feature/review" }])))
            },
            "The app answered without a task",
        ),
        (
            |method| (method == "task.link-issue").then(|| failure("linear")),
            "linear happened",
        ),
    ];
    for (answer, cause) in cases {
        let bridge = answering(answer);
        let mut run = triaged("ENG-7");
        run.variables.retain(|(name, _)| !name.ends_with(".repo"));
        let effect = TaskCreateEffect {
            from_remote: Vec::new(),
            ..review()
        };
        let (report, _) = run.publish(
            &mut publisher(&forge, &bridge),
            1,
            vec![Effect::TaskCreate(effect)],
        );
        assert_eq!(report.cause.as_deref(), Some(cause));
    }
}

#[test]
fn a_test_run_shows_what_the_run_decided_without_resolving_it() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(silent());
    let mut run = triaged("ENG-7");
    run.staged = true;

    let (_, effects) = run.publish(
        &mut publisher(&forge, &bridge),
        1,
        vec![
            Effect::TaskCreate(review()),
            Effect::TaskCreate(TaskCreateEffect {
                linear_issue: Some("".into()),
                ..TaskCreateEffect::default()
            }),
        ],
    );

    assert_eq!(
        effects[0].detail.as_deref(),
        Some(
            "Create or reuse the task on feature/review with web, api in Acme, checking out \
             origin/feature/review in api, linked to ENG-7"
        )
    );
    assert_eq!(
        effects[1].detail.as_deref(),
        Some("Create or reuse the task on wtm/demo/1")
    );
}

fn issue_task() -> TaskCreateEffect {
    TaskCreateEffect {
        repos: vec!["api".into()],
        linear_issue: Some("{steps.triage.result.issue}".into()),
        ..TaskCreateEffect::default()
    }
}

fn taskless(issue: &str) -> Run {
    let mut run = triaged(issue);
    run.taskless = true;
    run
}

#[test]
fn without_a_branch_or_task_it_creates_the_task_on_the_branch_linear_suggests_for_the_issue() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(tasks_bridge(Vec::new()));

    let (report, effects) = taskless("ENG-7").publish(
        &mut publisher(&forge, &bridge),
        1,
        vec![Effect::TaskCreate(issue_task())],
    );

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(
        effects[0].detail.as_deref(),
        Some("Created a task on me/eng-7-review; linked it to ENG-7")
    );
    let calls = bridge.calls();
    assert_eq!(
        call(&calls, "linear.issue"),
        &json!({ "workspaceId": "w1", "issue": "ENG-7" })
    );
    assert_eq!(
        call(&calls, "task.create")["input"]["branchName"],
        "me/eng-7-review"
    );
}

#[test]
fn the_task_already_on_the_issue_branch_is_reused() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(tasks_bridge(vec![listed_task(
        "t1",
        "me/eng-7-review",
        &["r1"],
        Some("ENG-7"),
    )]));

    let (report, effects) = taskless("ENG-7").publish(
        &mut publisher(&forge, &bridge),
        2,
        vec![Effect::TaskCreate(issue_task())],
    );

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(effects[0].status, EffectStatus::Reconciled);
    assert_eq!(effects[0].reference.as_deref(), Some("t1"));
    assert_eq!(
        methods(&bridge),
        ["workspace.list", "linear.issue", "task.list"]
    );
}

#[test]
fn without_a_branch_or_task_an_empty_issue_or_one_without_a_branch_fails_the_effect() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(tasks_bridge(Vec::new()));
    let (report, _) = taskless(" ").publish(
        &mut publisher(&forge, &bridge),
        1,
        vec![Effect::TaskCreate(issue_task())],
    );
    assert_eq!(
        report.cause.as_deref(),
        Some("linear_issue rendered no issue, and there is no branch to create the task on")
    );
    assert!(!methods(&bridge).contains(&"task.create".to_string()));

    let branchless = Arc::new(FakeBridge::answering(|method, params| match method {
        "linear.issue" => ok(json!({ "identifier": "ENG-7", "branchName": "" })),
        _ => (tasks_bridge(Vec::new()).answer)(method, params),
    }));
    let (report, _) = taskless("ENG-7").publish(
        &mut publisher(&forge, &branchless),
        1,
        vec![Effect::TaskCreate(issue_task())],
    );
    assert_eq!(
        report.cause.as_deref(),
        Some("Linear suggests no branch for ENG-7")
    );
}

#[test]
fn a_test_run_shows_a_placeholder_for_the_issue_branch() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(silent());
    let mut run = taskless("ENG-7");
    run.staged = true;

    let (_, effects) = run.publish(
        &mut publisher(&forge, &bridge),
        1,
        vec![Effect::TaskCreate(issue_task())],
    );

    assert_eq!(
        effects[0].detail.as_deref(),
        Some("Create or reuse the task on <issue branch> with api, linked to ENG-7")
    );
    assert!(bridge.calls().is_empty());
}
