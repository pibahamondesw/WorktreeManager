use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

use super::forge::tail;
use super::*;
use crate::commands::automations::definition::{Capacity, Step, StepKind, Transition};
use crate::commands::automations::store::events::{reduce, LoggedEvent, StepEffect, StepStatus};
use crate::commands::git::git_command;

const ITEM: &str = "routine";

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "wtm-publish-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(fs::canonicalize(dir).unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct FakeForge {
    head: String,
    remote_head: Option<String>,
    push_error: Option<String>,
    pr: Mutex<Option<PullRequest>>,
    commented: Vec<String>,
    dirty: Vec<String>,
    calls: Mutex<Vec<String>>,
}

impl FakeForge {
    fn with_pr(draft: bool) -> Self {
        Self {
            pr: Mutex::new(Some(pull_request(draft))),
            ..Self::at("abc")
        }
    }

    fn at(head: &str) -> Self {
        Self {
            head: head.into(),
            ..Self::default()
        }
    }

    fn call(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

fn pull_request(draft: bool) -> PullRequest {
    PullRequest {
        number: 7,
        url: "https://github.com/o/r/pull/7".into(),
        title: "Old".into(),
        body: "Old body".into(),
        draft,
    }
}

impl Forge for FakeForge {
    fn head(&self, _: &Path) -> Result<String, String> {
        Ok(self.head.clone())
    }

    fn remote_head(&self, _: &Path, branch: &str) -> Result<Option<String>, String> {
        self.call(format!("remote_head {branch}"));
        Ok(self.remote_head.clone())
    }

    fn push(&self, worktree: &Path, branch: &str, timeout: Duration) -> Result<(), String> {
        self.call(format!(
            "push {} {branch} {}",
            worktree.display(),
            timeout.as_secs()
        ));
        self.push_error.clone().map_or(Ok(()), Err)
    }

    fn last_subject(&self, _: &Path) -> Result<String, String> {
        Ok("Fix the offenses".into())
    }

    fn dirty_paths(&self, worktree: &Path) -> Result<Vec<String>, String> {
        self.call(format!("dirty {}", worktree.display()));
        Ok(self.dirty.clone())
    }

    fn open_pull_request(&self, _: &Path, branch: &str) -> Result<Option<PullRequest>, String> {
        self.call(format!("open {branch}"));
        Ok(self.pr.lock().unwrap().clone())
    }

    fn create_pull_request(&self, _: &Path, new: &NewPullRequest) -> Result<PullRequest, String> {
        self.call(format!("create {new:?}"));
        Ok(pull_request(new.draft))
    }

    fn edit_pull_request(
        &self,
        _: &Path,
        number: u64,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<(), String> {
        self.call(format!("edit {number} {title:?} {body:?}"));
        Ok(())
    }

    fn set_draft(&self, _: &Path, number: u64, draft: bool) -> Result<(), String> {
        self.call(format!("draft {number} {draft}"));
        Ok(())
    }

    fn has_comment(&self, _: &Path, _: u64, marker: &str) -> Result<bool, String> {
        Ok(self
            .commented
            .iter()
            .any(|comment| comment.contains(marker)))
    }

    fn comment(&self, _: &Path, number: u64, body: &str) -> Result<(), String> {
        self.call(format!("comment {number} {body}"));
        Ok(())
    }
}

type Answer = Box<dyn Fn(&str, &Value) -> Value + Send + Sync>;

struct FakeBridge {
    answer: Answer,
    calls: Mutex<Vec<(String, Value)>>,
}

impl FakeBridge {
    fn answering(answer: impl Fn(&str, &Value) -> Value + Send + Sync + 'static) -> Self {
        Self {
            answer: Box::new(answer),
            calls: Mutex::default(),
        }
    }

    fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }
}

impl OperationsBridge for FakeBridge {
    fn call(&self, method: &str, params: Value) -> Value {
        let response = (self.answer)(method, &params);
        self.calls.lock().unwrap().push((method.into(), params));
        response
    }
}

fn ok(data: Value) -> Value {
    json!({ "ok": true, "data": data, "warnings": [] })
}

fn failure(code: &str) -> Value {
    json!({ "ok": false, "error": { "code": code, "message": format!("{code} happened") } })
}

fn silent() -> FakeBridge {
    FakeBridge::answering(|method, _| panic!("unexpected call to {method}"))
}

fn step(effects: Vec<Effect>) -> Step {
    Step {
        id: "publish".into(),
        label: "publish".into(),
        next: None,
        on_fail: Transition::Fail,
        timeout_secs: Some(90),
        capacity: Capacity::Hold,
        kind: StepKind::Publish(effects),
    }
}

fn task(worktree: &Path) -> ResolvedTask {
    ResolvedTask {
        id: "t1".into(),
        branch: "wtm/demo/1".into(),
        folders: vec![worktree.to_string_lossy().into_owned()],
        repo_ids: vec!["r1".into()],
    }
}

fn allowing(verbs: &[Verb]) -> Authorization {
    Authorization {
        allow: verbs.to_vec(),
        deny: Vec::new(),
        caps: BTreeMap::new(),
    }
}

fn tracker() -> LinearTracker {
    LinearTracker {
        team: "DX".into(),
        project: Some("Lint".into()),
        assignee: "me".into(),
        labels: vec!["Code quality".into()],
        title: "Lint: {item.id}".into(),
        marker: "lint:{item.id}".into(),
        states: BTreeMap::from([("review".into(), "In Review".into())]),
    }
}

fn pr_effect(title: Option<&str>) -> PullRequestEffect {
    PullRequestEffect {
        repo: "r1".into(),
        title: title.map(String::from),
        body_template: None,
        lang: None,
        draft: None,
        assignee: None,
    }
}

struct Run {
    worktree: TempDir,
    folder: TempDir,
    authorization: Authorization,
    tracker: Option<LinearTracker>,
    staged: bool,
    stale: Vec<String>,
    bases: BTreeMap<String, String>,
}

impl Run {
    fn allowing(verbs: &[Verb]) -> Self {
        Self {
            worktree: TempDir::new("worktree"),
            folder: TempDir::new("folder"),
            authorization: allowing(verbs),
            tracker: Some(tracker()),
            staged: false,
            stale: Vec::new(),
            bases: BTreeMap::new(),
        }
    }

    fn all() -> Self {
        Self::allowing(&Verb::ALL)
    }

    fn publish(
        &self,
        publisher: &mut Publisher,
        attempt: u32,
        effects: Vec<Effect>,
    ) -> (StepReport, Vec<StepEffect>) {
        let step = step(effects.clone());
        let request = StepRequest {
            item: ITEM,
            step: &step,
            attempt,
            stage: None,
            active_time_left: Duration::from_secs(60),
        };
        let task = task(&self.worktree.0);
        let variables = vec![
            ("automation".to_string(), "demo".to_string()),
            ("item.id".to_string(), ITEM.to_string()),
            ("task.branch".to_string(), task.branch.clone()),
        ];
        let stale_receipts = |repo: &str| match repo {
            "broken" => Err("receipts unreadable".to_string()),
            _ => Ok(self.stale.clone()),
        };
        let repo_names = BTreeMap::from([("r1".to_string(), "api".to_string())]);
        let scope = PublishScope {
            automation: "demo",
            run_id: "20261103-1",
            workspace_id: "w1",
            folder: &self.folder.0,
            task: &task,
            repo_names: &repo_names,
            bases: &self.bases,
            tracker: self.tracker.as_ref(),
            authorization: &self.authorization,
            staged: self.staged,
            variables: &variables,
            stale_receipts: &stale_receipts,
        };
        let report = publisher.publish(&request, &effects, &scope);
        let mut events = vec![
            RunEvent::ItemCreated {
                item: ITEM.into(),
                version: "1".into(),
                title: None,
                data: Default::default(),
            },
            RunEvent::StepStatus {
                item: ITEM.into(),
                step: "publish".into(),
                attempt,
                status: StepStatus::Running,
                process_end: None,
                exit_code: None,
                outcome: None,
                stage: None,
                verdict: None,
                cause: None,
                head: None,
            },
        ];
        events.extend(publisher.drain_events());
        let run = reduce(
            "run",
            events
                .into_iter()
                .map(|event| LoggedEvent { at_ms: 0, event }),
        );
        (report, run.items[0].steps[0].effects.clone())
    }
}

fn publisher(forge: &Arc<FakeForge>, bridge: &Arc<FakeBridge>) -> Publisher {
    Publisher::new(forge.clone(), bridge.clone())
}

fn statuses(effects: &[StepEffect]) -> Vec<EffectStatus> {
    effects.iter().map(|effect| effect.status).collect()
}

fn push() -> Effect {
    Effect::GitPush { repo: "r1".into() }
}

#[test]
fn effects_whose_verb_is_not_allowed_or_is_denied_block_the_item() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(silent());
    let mut run = Run::allowing(&[Verb::Push]);
    let (report, effects) =
        run.publish(&mut publisher(&forge, &bridge), 1, vec![Effect::TaskCreate]);
    assert_eq!(report.verdict, Some(Verdict::Blocked));
    assert_eq!(report.cause.as_deref(), Some("unauthorized:task_create"));
    assert_eq!(statuses(&effects), [EffectStatus::Denied]);

    run.authorization.deny = vec![Verb::Push];
    let (report, effects) = run.publish(&mut publisher(&forge, &bridge), 1, vec![push()]);
    assert_eq!(report.cause.as_deref(), Some("unauthorized:push"));
    assert_eq!(effects[0].detail.as_deref(), Some("push is not allowed"));
    assert!(forge.calls().is_empty());
}

#[test]
fn caps_count_the_effects_each_verb_ran_for_the_item() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(silent());
    let mut run = Run::allowing(&[Verb::Push]);
    run.authorization.caps = BTreeMap::from([(Verb::Push, 2)]);
    let mut publisher = publisher(&forge, &bridge);

    let (report, _) = run.publish(&mut publisher, 1, vec![push(), push()]);
    assert_eq!(report.outcome, StepOutcome::Success);
    let (report, effects) = run.publish(&mut publisher, 2, vec![push()]);

    assert_eq!(report.verdict, Some(Verdict::Blocked));
    assert_eq!(report.cause.as_deref(), Some("cap:push"));
    assert_eq!(effects[0].status, EffectStatus::Capped);
    assert_eq!(effects[0].detail.as_deref(), Some("2 per item reached"));
}

#[test]
fn reconciled_effects_do_not_count_toward_the_cap() {
    let forge = Arc::new(FakeForge {
        remote_head: Some("abc".into()),
        ..FakeForge::at("abc")
    });
    let bridge = Arc::new(silent());
    let mut run = Run::allowing(&[Verb::Push]);
    run.authorization.caps = BTreeMap::from([(Verb::Push, 1)]);

    let (report, effects) = run.publish(&mut publisher(&forge, &bridge), 1, vec![push(), push()]);

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(statuses(&effects), [EffectStatus::Reconciled; 2]);
    assert_eq!(effects[0].reference.as_deref(), Some("abc"));
}

#[test]
fn test_runs_stage_every_effect_without_running_any() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(silent());
    let mut run = Run::all();
    run.staged = true;
    let mut update = pr_effect(Some("[{tracker.issue}] Lint"));
    update.body_template = Some("templates/pr.md".into());
    update.draft = Some(false);
    let effects = vec![
        push(),
        Effect::PrCreate(pr_effect(None)),
        Effect::PrCreate(PullRequestEffect {
            draft: Some(false),
            ..pr_effect(Some("Ready {pr.url}"))
        }),
        Effect::PrUpdate(update),
        Effect::PrComment {
            repo: "r1".into(),
            body: "See {pr.url}".into(),
        },
        Effect::PrReady { repo: "r1".into() },
        Effect::LinearUpsert(LinearOverrides::default()),
        Effect::LinearState {
            state: "review".into(),
        },
        Effect::TaskCreate,
        Effect::TaskLog {
            entry: "Opened {pr.url}".into(),
        },
        Effect::TaskDeleteClean,
    ];

    let (report, effects) = run.publish(&mut publisher(&forge, &bridge), 1, effects);

    assert_eq!(report.outcome, StepOutcome::Success);
    assert!(effects
        .iter()
        .all(|effect| effect.status == EffectStatus::Staged));
    let details: Vec<_> = effects
        .iter()
        .map(|effect| effect.detail.clone().unwrap())
        .collect();
    assert_eq!(
        details,
        [
            "Push wtm/demo/1 of api to origin",
            "Open a draft PR for wtm/demo/1: the last commit's subject",
            "Open a ready PR for wtm/demo/1: Ready <PR URL>",
            "Update the PR for wtm/demo/1: title [<issue id>] Lint, body from templates/pr.md, draft false",
            "Comment on the PR for wtm/demo/1: See <PR URL>",
            "Mark the PR for wtm/demo/1 ready for review",
            "Create or update the DX issue marked lint:routine: Lint: routine",
            "Move the issue marked lint:routine to In Review",
            "Create or reuse the task on wtm/demo/1",
            "Append to the task log: Opened <PR URL>",
            "Delete task t1 and its worktrees if they are clean",
        ]
    );
    assert!(forge.calls().is_empty());
}

#[test]
fn a_staged_effect_that_cannot_render_fails_the_step() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(silent());
    let mut run = Run::all();
    run.staged = true;
    run.tracker = None;

    let (report, effects) = run.publish(
        &mut publisher(&forge, &bridge),
        1,
        vec![Effect::LinearUpsert(LinearOverrides::default())],
    );

    assert_eq!(report.outcome, StepOutcome::Failure);
    assert_eq!(
        report.cause.as_deref(),
        Some("Linear effects need tracker.linear")
    );
    assert_eq!(statuses(&effects), [EffectStatus::Failed]);
}

#[test]
fn pushes_reconcile_with_the_remote_and_never_run_on_stale_receipts() {
    let bridge = Arc::new(silent());
    let mut run = Run::allowing(&[Verb::Push]);
    let forge = Arc::new(FakeForge {
        remote_head: Some("old".into()),
        ..FakeForge::at("abc")
    });

    let (report, effects) = run.publish(&mut publisher(&forge, &bridge), 1, vec![push()]);
    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(effects[0].status, EffectStatus::Executed);
    let worktree = run.worktree.0.display();
    assert_eq!(
        forge.calls(),
        [
            "remote_head wtm/demo/1".to_string(),
            format!("push {worktree} wtm/demo/1 90")
        ]
    );

    run.stale = vec!["verify".into(), "lint".into()];
    let (report, effects) = run.publish(&mut publisher(&forge, &bridge), 2, vec![push()]);
    assert_eq!(report.cause.as_deref(), Some("stale_receipt:verify"));
    assert_eq!(
        effects[0].detail.as_deref(),
        Some("The receipts of verify, lint no longer match the worktree; run them again first")
    );

    let broken = Effect::GitPush {
        repo: "broken".into(),
    };
    let (report, _) = run.publish(&mut publisher(&forge, &bridge), 3, vec![broken]);
    assert_eq!(report.cause.as_deref(), Some("receipts unreadable"));
}

#[test]
fn a_push_hooks_reject_fails_the_step_with_their_output() {
    let bridge = Arc::new(silent());
    let run = Run::allowing(&[Verb::Push]);
    let forge = Arc::new(FakeForge {
        push_error: Some("git push failed: pre-push: lint".into()),
        ..FakeForge::at("abc")
    });

    let (report, effects) = run.publish(&mut publisher(&forge, &bridge), 1, vec![push(), push()]);

    assert_eq!(
        report.cause.as_deref(),
        Some("git push failed: pre-push: lint")
    );
    assert_eq!(effects.len(), 1, "later effects do not run");
}

#[test]
fn effects_on_a_repository_without_a_worktree_fail() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(silent());
    let run = Run::all();
    let effect = Effect::PrReady { repo: "r9".into() };

    let (report, _) = run.publish(&mut publisher(&forge, &bridge), 1, vec![effect]);

    assert_eq!(
        report.cause.as_deref(),
        Some("task t1 has no worktree for repository r9")
    );
}

#[test]
fn pr_create_opens_a_draft_assigned_to_the_owner_with_the_repository_template() {
    let forge = Arc::new(FakeForge::at("abc"));
    let run = Run::all();
    fs::create_dir_all(run.worktree.0.join(".github")).unwrap();
    fs::write(
        run.worktree.0.join(".github/pull_request_template.md"),
        "## Context {verbatim}",
    )
    .unwrap();
    fs::write(
        run.worktree.0.join(".github/pull_request_template.es.md"),
        "## Contexto {verbatim}",
    )
    .unwrap();
    let effects = vec![
        Effect::PrCreate(PullRequestEffect {
            lang: Some("es".into()),
            ..pr_effect(None)
        }),
        Effect::TaskLog {
            entry: "Opened {pr.url} (#{pr.number})".into(),
        },
    ];
    let bridge_log = Arc::new(FakeBridge::answering(|_, _| {
        ok(json!({ "status": "appended", "path": "/vault/t1.md" }))
    }));

    let (report, effects) = run.publish(&mut publisher(&forge, &bridge_log), 1, effects);

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(
        effects[0].reference.as_deref(),
        Some("https://github.com/o/r/pull/7")
    );
    assert_eq!(
        forge.calls()[1],
        "create NewPullRequest { branch: \"wtm/demo/1\", title: \"Fix the offenses\", body: \"## Contexto {verbatim}\", draft: true, assignee: \"@me\" }"
    );
    assert_eq!(
        bridge_log.calls()[0].1["entry"],
        "Opened https://github.com/o/r/pull/7 (#7)"
    );
}

#[test]
fn pr_create_without_a_repository_template_opens_an_empty_body() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(silent());
    let run = Run::all();
    let effect = Effect::PrCreate(PullRequestEffect {
        draft: Some(false),
        assignee: Some("octocat".into()),
        ..pr_effect(Some("Lint {item.id}"))
    });

    run.publish(&mut publisher(&forge, &bridge), 1, vec![effect]);

    assert_eq!(
        forge.calls()[1],
        "create NewPullRequest { branch: \"wtm/demo/1\", title: \"Lint routine\", body: \"\", draft: false, assignee: \"octocat\" }"
    );
}

#[test]
fn a_retried_pr_create_reuses_the_open_pr_and_only_retitles_it() {
    let forge = Arc::new(FakeForge::with_pr(true));
    let bridge = Arc::new(silent());
    let run = Run::all();
    let effect = Effect::PrCreate(PullRequestEffect {
        body_template: Some("missing.md".into()),
        draft: Some(false),
        ..pr_effect(Some("New"))
    });

    let (report, effects) = run.publish(&mut publisher(&forge, &bridge), 2, vec![effect]);

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(effects[0].status, EffectStatus::Executed);
    assert_eq!(effects[0].detail.as_deref(), Some("Updated #7: title"));
    assert_eq!(
        forge.calls(),
        [
            "open wtm/demo/1",
            "open wtm/demo/1",
            "edit 7 Some(\"New\") None"
        ]
    );

    let untitled = Effect::PrCreate(pr_effect(None));
    let (_, effects) = run.publish(&mut publisher(&forge, &bridge), 3, vec![untitled]);
    assert_eq!(effects[0].status, EffectStatus::Reconciled);
    assert_eq!(effects[0].detail.as_deref(), Some("#7 is up to date"));
}

#[test]
fn pr_update_renders_the_automation_template_in_its_language() {
    let forge = Arc::new(FakeForge::with_pr(false));
    let bridge = Arc::new(silent());
    let run = Run::all();
    fs::create_dir_all(run.folder.0.join("templates")).unwrap();
    fs::write(run.folder.0.join("templates/pr.md"), "Body {item.id}").unwrap();
    fs::write(run.folder.0.join("templates/pr.es.md"), "Cuerpo {item.id}").unwrap();
    let effect = Effect::PrUpdate(PullRequestEffect {
        body_template: Some("templates/pr.md".into()),
        lang: Some("es".into()),
        draft: Some(true),
        ..pr_effect(Some("Old"))
    });

    let (report, effects) = run.publish(&mut publisher(&forge, &bridge), 1, vec![effect]);

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(
        effects[0].detail.as_deref(),
        Some("Updated #7: body, draft")
    );
    assert_eq!(
        forge.calls()[1..],
        ["edit 7 None Some(\"Cuerpo routine\")", "draft 7 true"]
    );
}

#[test]
fn pr_update_needs_an_open_pr_and_a_readable_template() {
    let bridge = Arc::new(silent());
    let run = Run::all();
    let none = Arc::new(FakeForge::at("abc"));
    let (report, _) = run.publish(
        &mut publisher(&none, &bridge),
        1,
        vec![Effect::PrUpdate(pr_effect(Some("New")))],
    );
    assert_eq!(report.cause.as_deref(), Some("No open PR for wtm/demo/1"));

    let open = Arc::new(FakeForge::with_pr(false));
    let effect = Effect::PrUpdate(PullRequestEffect {
        body_template: Some("gone.md".into()),
        ..pr_effect(None)
    });
    let (report, _) = run.publish(&mut publisher(&open, &bridge), 1, vec![effect]);
    assert!(report.cause.unwrap().contains("gone.md"));
}

#[test]
fn pr_comments_are_posted_once_per_head() {
    let bridge = Arc::new(silent());
    let run = Run::all();
    let comment = || Effect::PrComment {
        repo: "r1".into(),
        body: "fin review".into(),
    };
    let fresh = Arc::new(FakeForge::with_pr(false));
    let (_, effects) = run.publish(&mut publisher(&fresh, &bridge), 1, vec![comment()]);
    assert_eq!(effects[0].status, EffectStatus::Executed);
    assert_eq!(
        fresh.calls()[1],
        "comment 7 fin review\n\n<!-- wtm:demo:routine:publish:abc -->"
    );

    let commented = Arc::new(FakeForge {
        commented: vec!["fin review\n\n<!-- wtm:demo:routine:publish:abc -->".into()],
        ..FakeForge::with_pr(false)
    });
    let (_, effects) = run.publish(&mut publisher(&commented, &bridge), 2, vec![comment()]);
    assert_eq!(effects[0].status, EffectStatus::Reconciled);
}

#[test]
fn pr_ready_marks_drafts_only() {
    let bridge = Arc::new(silent());
    let run = Run::all();
    let ready = || Effect::PrReady { repo: "r1".into() };

    let draft = Arc::new(FakeForge::with_pr(true));
    let (_, effects) = run.publish(&mut publisher(&draft, &bridge), 1, vec![ready()]);
    assert_eq!(effects[0].status, EffectStatus::Executed);
    assert_eq!(draft.calls()[1], "draft 7 false");

    let open = Arc::new(FakeForge::with_pr(false));
    let (_, effects) = run.publish(&mut publisher(&open, &bridge), 1, vec![ready()]);
    assert_eq!(effects[0].status, EffectStatus::Reconciled);
}

fn linear_bridge(created: bool, updated: bool, changed: bool) -> FakeBridge {
    FakeBridge::answering(move |method, _| {
        if method == "linear.issue-upsert" {
            ok(json!({ "identifier": "DX-9", "created": created, "updated": updated }))
        } else {
            ok(json!({ "identifier": "DX-9", "changed": changed }))
        }
    })
}

#[test]
fn linear_upsert_merges_the_tracker_and_names_the_issue_for_later_effects() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(linear_bridge(true, false, true));
    let run = Run::all();
    let overrides = LinearOverrides {
        title: Some("Override {item.id}".into()),
        marker: Some("custom:{item.id}".into()),
        labels: Some(Vec::new()),
        ..LinearOverrides::default()
    };
    let effects = vec![
        Effect::LinearUpsert(overrides),
        Effect::LinearState {
            state: "review".into(),
        },
        Effect::PrCreate(pr_effect(Some("[{tracker.issue}] Lint"))),
    ];

    let (report, effects) = run.publish(&mut publisher(&forge, &bridge), 1, effects);

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(
        statuses(&effects),
        [
            EffectStatus::Executed,
            EffectStatus::Executed,
            EffectStatus::Executed
        ]
    );
    assert_eq!(effects[0].detail.as_deref(), Some("Created DX-9"));
    assert_eq!(
        effects[1].detail.as_deref(),
        Some("Moved DX-9 to In Review")
    );
    let calls = bridge.calls();
    assert_eq!(
        calls[0].1,
        json!({
            "workspaceId": "w1",
            "team": "DX",
            "project": "Lint",
            "assignee": "me",
            "labels": [],
            "title": "Override routine",
            "marker": "custom:routine",
        })
    );
    assert_eq!(
        calls[1].1,
        json!({ "workspaceId": "w1", "marker": "custom:routine", "state": "In Review" })
    );
    assert!(forge.calls()[1].contains("title: \"[DX-9] Lint\""));
}

#[test]
fn linear_effects_reconcile_with_the_issue_the_marker_finds() {
    let forge = Arc::new(FakeForge::at("abc"));
    let run = Run::all();
    let effects = || {
        vec![
            Effect::LinearUpsert(LinearOverrides::default()),
            Effect::LinearState {
                state: "Done".into(),
            },
        ]
    };

    let unchanged = Arc::new(linear_bridge(false, false, false));
    let (_, effects_run) = run.publish(&mut publisher(&forge, &unchanged), 1, effects());
    assert_eq!(statuses(&effects_run), [EffectStatus::Reconciled; 2]);
    assert_eq!(effects_run[1].detail.as_deref(), Some("DX-9 is in Done"));
    assert_eq!(unchanged.calls()[1].1["marker"], "lint:routine");

    let retitled = Arc::new(linear_bridge(false, true, false));
    let (_, effects_run) = run.publish(&mut publisher(&forge, &retitled), 1, effects());
    assert_eq!(effects_run[0].detail.as_deref(), Some("Updated DX-9"));
}

#[test]
fn linear_state_finds_the_issue_by_the_tracker_marker_without_an_upsert() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(linear_bridge(false, false, true));
    let run = Run::all();

    run.publish(
        &mut publisher(&forge, &bridge),
        1,
        vec![Effect::LinearState {
            state: "started".into(),
        }],
    );

    assert_eq!(
        bridge.calls()[0].1,
        json!({ "workspaceId": "w1", "marker": "lint:routine", "state": "started" })
    );
}

#[test]
fn linear_failures_fail_the_step_with_the_operation_message() {
    let forge = Arc::new(FakeForge::at("abc"));
    let run = Run::all();
    let upsert = || vec![Effect::LinearUpsert(LinearOverrides::default())];

    let unconfigured = Arc::new(FakeBridge::answering(|_, _| {
        failure("linear_not_configured")
    }));
    let (report, _) = run.publish(&mut publisher(&forge, &unconfigured), 1, upsert());
    assert_eq!(
        report.cause.as_deref(),
        Some("linear_not_configured happened")
    );

    let bare = Arc::new(FakeBridge::answering(
        |_, _| json!({ "ok": false, "error": { "code": "app_closed" } }),
    ));
    let (report, _) = run.publish(&mut publisher(&forge, &bare), 1, upsert());
    assert_eq!(report.cause.as_deref(), Some("app_closed"));

    let empty = Arc::new(FakeBridge::answering(|_, _| json!({ "ok": false })));
    let (report, _) = run.publish(&mut publisher(&forge, &empty), 1, upsert());
    assert_eq!(report.cause.as_deref(), Some("The operation failed"));

    let anonymous = Arc::new(FakeBridge::answering(|_, _| ok(json!({}))));
    let (report, _) = run.publish(&mut publisher(&forge, &anonymous), 1, upsert());
    assert_eq!(
        report.cause.as_deref(),
        Some("The app answered without identifier")
    );
}

#[test]
fn task_create_reuses_the_task_on_the_branch() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(FakeBridge::answering(|_, _| {
        ok(
            json!([{ "id": "t0", "branchName": "other" }, { "id": "t1", "branchName": "wtm/demo/1" }]),
        )
    }));
    let run = Run::all();

    let (_, effects) = run.publish(&mut publisher(&forge, &bridge), 1, vec![Effect::TaskCreate]);

    assert_eq!(effects[0].status, EffectStatus::Reconciled);
    assert_eq!(effects[0].reference.as_deref(), Some("t1"));
    assert_eq!(
        bridge.calls()[0],
        ("task.list".into(), json!({ "workspaceId": "w1" }))
    );
}

#[test]
fn task_create_creates_the_task_and_keeps_setup_warnings() {
    let forge = Arc::new(FakeForge::at("abc"));
    let bridge = Arc::new(FakeBridge::answering(|method, _| match method {
        "task.list" => ok(json!([])),
        _ => json!({
            "ok": true,
            "data": { "id": "t2" },
            "warnings": [{ "stage": "open", "message": "Could not open the editor" }, {}]
        }),
    }));
    let mut run = Run::all();
    run.bases = BTreeMap::from([("r1".into(), "origin/develop".into())]);

    let (report, effects) =
        run.publish(&mut publisher(&forge, &bridge), 1, vec![Effect::TaskCreate]);

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(effects[0].reference.as_deref(), Some("t2"));
    assert_eq!(effects[0].warnings, ["Could not open the editor"]);
    assert_eq!(
        bridge.calls()[1].1,
        json!({ "input": {
            "workspaceId": "w1",
            "branchName": "wtm/demo/1",
            "repoIds": ["r1"],
            "automationId": "demo",
            "baseRefs": { "r1": "origin/develop" },
        } })
    );

    let without_bases = Run::all();
    without_bases.publish(&mut publisher(&forge, &bridge), 1, vec![Effect::TaskCreate]);
    assert!(bridge.calls()[3].1["input"].get("baseRefs").is_none());
}

#[test]
fn task_log_entries_are_keyed_per_attempt() {
    let forge = Arc::new(FakeForge::at("abc"));
    let run = Run::all();
    let log = || {
        vec![Effect::TaskLog {
            entry: "Integrated {item.id}".into(),
        }]
    };
    for (status, expected) in [
        ("appended", EffectStatus::Executed),
        ("present", EffectStatus::Reconciled),
        ("vault_disabled", EffectStatus::Skipped),
    ] {
        let bridge = Arc::new(FakeBridge::answering(move |_, _| {
            ok(json!({ "status": status, "path": "/vault/t1.md" }))
        }));
        let (_, effects) = run.publish(&mut publisher(&forge, &bridge), 4, log());
        assert_eq!(effects[0].status, expected);
        assert_eq!(
            bridge.calls()[0],
            (
                "task.log-append".into(),
                json!({
                    "id": "t1",
                    "entry": "Integrated routine",
                    "key": "wtm:demo:20261103-1:routine:publish:4"
                })
            )
        );
    }
}

#[test]
fn task_delete_clean_deletes_only_clean_tasks_and_keeps_cleanup_warnings() {
    let run = Run::all();
    let bridge = Arc::new(FakeBridge::answering(|method, _| match method {
        "task.get" => ok(json!({ "id": "t1" })),
        _ => json!({
            "ok": true,
            "data": { "id": "t1" },
            "warnings": [{ "stage": "archive_note", "message": "archive_note did not complete" }]
        }),
    }));
    let clean = Arc::new(FakeForge::at("abc"));

    let (report, effects) = run.publish(
        &mut publisher(&clean, &bridge),
        1,
        vec![Effect::TaskDeleteClean],
    );

    assert_eq!(report.outcome, StepOutcome::Success);
    assert_eq!(effects[0].status, EffectStatus::Executed);
    assert_eq!(effects[0].warnings, ["archive_note did not complete"]);
    assert_eq!(
        bridge.calls()[1],
        (
            "task.delete".into(),
            json!({ "id": "t1", "deleteWorktrees": true })
        )
    );

    let dirty = Arc::new(FakeForge {
        dirty: vec!["a.rb".into()],
        ..FakeForge::at("abc")
    });
    let (report, effects) = run.publish(
        &mut publisher(&dirty, &bridge),
        2,
        vec![Effect::TaskDeleteClean],
    );
    assert_eq!(report.verdict, Some(Verdict::Blocked));
    assert_eq!(report.cause.as_deref(), Some("dirty_task"));
    assert_eq!(
        effects[0].detail,
        Some(format!("{}/a.rb", run.worktree.0.display()))
    );
    assert_eq!(bridge.calls().len(), 3, "the dirty task is never deleted");
}

#[test]
fn task_delete_clean_reconciles_a_deleted_task_and_fails_on_other_errors() {
    let forge = Arc::new(FakeForge::at("abc"));
    let run = Run::all();
    let gone = Arc::new(FakeBridge::answering(|_, _| failure("not_found")));
    let (_, effects) = run.publish(
        &mut publisher(&forge, &gone),
        1,
        vec![Effect::TaskDeleteClean],
    );
    assert_eq!(effects[0].status, EffectStatus::Reconciled);

    let closed = Arc::new(FakeBridge::answering(|_, _| failure("app_closed")));
    let (report, _) = run.publish(
        &mut publisher(&forge, &closed),
        1,
        vec![Effect::TaskDeleteClean],
    );
    assert_eq!(report.cause.as_deref(), Some("app_closed happened"));
}

// ---- CliForge, against real repositories and a stand-in for gh ----

const FAKE_GH: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_GH_DIR/calls"
[ -z "$FAKE_GH_FAIL" ] || { echo "gh: not logged in" >&2; exit 1; }
case "$1 $2" in
  "pr list") cat "$FAKE_GH_DIR/list.json" ;;
  "pr create") cp "$FAKE_GH_DIR/created.json" "$FAKE_GH_DIR/list.json"; echo "https://github.com/o/r/pull/7" ;;
  "pr view") cat "$FAKE_GH_DIR/view.json" ;;
esac
"#;

const PR_JSON: &str =
    r#"[{"number":7,"url":"https://github.com/o/r/pull/7","title":"T","isDraft":true}]"#;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = git_command()
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn isolated_git() -> Vec<(String, String)> {
    vec![
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ]
}

/// A clone of an empty bare remote with one commit on `main`.
fn clone_with_remote() -> (TempDir, TempDir) {
    let remote = TempDir::new("remote");
    git(&remote.0, &["init", "-q", "--bare", "-b", "main"]);
    let clone = TempDir::new("clone");
    git(&clone.0, &["init", "-q", "-b", "main"]);
    git(&clone.0, &["config", "user.email", "wtm@example.com"]);
    git(&clone.0, &["config", "user.name", "WTM"]);
    git(
        &clone.0,
        &["remote", "add", "origin", &remote.0.to_string_lossy()],
    );
    fs::write(clone.0.join("a.txt"), "one").unwrap();
    git(&clone.0, &["add", "a.txt"]);
    git(&clone.0, &["commit", "-q", "-m", "First change"]);
    (remote, clone)
}

fn hook(clone: &Path, script: &str) {
    let path = clone.join(".git/hooks/pre-push");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn cli_forge_reads_and_pushes_the_worktree_with_its_hooks() {
    let (remote, clone) = clone_with_remote();
    let forge = CliForge::with("gh", isolated_git());
    let head = forge.head(&clone.0).unwrap();
    assert_eq!(forge.last_subject(&clone.0).unwrap(), "First change");
    assert_eq!(forge.remote_head(&clone.0, "wtm/x").unwrap(), None);
    hook(&clone.0, "#!/bin/sh\necho hook ran >&2\nexit 1\n");

    let rejected = forge
        .push(&clone.0, "wtm/x", Duration::from_secs(60))
        .unwrap_err();
    assert!(rejected.contains("hook ran"), "{rejected}");

    hook(&clone.0, "#!/bin/sh\nexit 0\n");
    forge
        .push(&clone.0, "wtm/x", Duration::from_secs(60))
        .unwrap();
    assert_eq!(
        forge.remote_head(&clone.0, "wtm/x").unwrap(),
        Some(head.clone())
    );
    assert_eq!(git(&remote.0, &["rev-parse", "wtm/x"]), head);

    fs::write(clone.0.join("a.txt"), "two").unwrap();
    fs::write(clone.0.join("b.txt"), "new").unwrap();
    assert_eq!(forge.dirty_paths(&clone.0).unwrap(), ["a.txt", "b.txt"]);
}

#[test]
fn cli_forge_stops_a_push_whose_hooks_outlast_the_timeout() {
    let (_remote, clone) = clone_with_remote();
    hook(&clone.0, "#!/bin/sh\nsleep 30\n");
    let forge = CliForge::with("gh", isolated_git());

    let error = forge
        .push(&clone.0, "wtm/x", Duration::from_millis(1500))
        .unwrap_err();

    assert!(error.starts_with("git push timed out"), "{error}");
}

#[test]
fn cli_forge_reports_git_failures() {
    let outside = TempDir::new("outside");
    let forge = CliForge::default();
    assert!(forge
        .head(&outside.0)
        .unwrap_err()
        .starts_with("git rev-parse"));
    assert!(forge
        .remote_head(&outside.0, "main")
        .unwrap_err()
        .starts_with("git ls-remote"));
}

struct FakeGh {
    dir: TempDir,
    forge: CliForge,
}

impl FakeGh {
    fn new(fail: bool) -> Self {
        let dir = TempDir::new("gh");
        let program = dir.0.join("gh");
        fs::write(&program, FAKE_GH).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        let mut env = vec![(
            "FAKE_GH_DIR".to_string(),
            dir.0.to_string_lossy().into_owned(),
        )];
        if fail {
            env.push(("FAKE_GH_FAIL".into(), "1".into()));
        }
        let forge = CliForge::with(&program.to_string_lossy(), env);
        Self { dir, forge }
    }

    fn write(&self, file: &str, contents: &str) {
        fs::write(self.dir.0.join(file), contents).unwrap();
    }

    fn calls(&self) -> Vec<String> {
        let calls = fs::read_to_string(self.dir.0.join("calls")).unwrap_or_default();
        calls.lines().map(String::from).collect()
    }
}

#[test]
fn cli_forge_finds_creates_and_edits_pull_requests_with_gh() {
    let gh = FakeGh::new(false);
    let worktree = &gh.dir.0;
    gh.write("list.json", "[]");
    gh.write("created.json", PR_JSON);
    assert_eq!(gh.forge.open_pull_request(worktree, "wtm/x").unwrap(), None);
    let new = NewPullRequest {
        branch: "wtm/x".into(),
        title: "T".into(),
        body: "B".into(),
        draft: true,
        assignee: "@me".into(),
    };

    let created = gh.forge.create_pull_request(worktree, &new).unwrap();
    gh.forge
        .edit_pull_request(worktree, 7, Some("T2"), Some("B2"))
        .unwrap();
    gh.forge.edit_pull_request(worktree, 7, None, None).unwrap();
    gh.forge.set_draft(worktree, 7, true).unwrap();
    gh.forge.set_draft(worktree, 7, false).unwrap();
    gh.forge.comment(worktree, 7, "hi").unwrap();

    assert_eq!(
        created,
        PullRequest {
            number: 7,
            url: "https://github.com/o/r/pull/7".into(),
            title: "T".into(),
            body: String::new(),
            draft: true,
        }
    );
    assert_eq!(
        gh.calls()[1..],
        [
            "pr create --head wtm/x --title T --body B --assignee @me --draft",
            "pr list --head wtm/x --state open --limit 1 --json number,url,title,body,isDraft",
            "pr edit 7 --title T2 --body B2",
            "pr edit 7",
            "pr ready 7 --undo",
            "pr ready 7",
            "pr comment 7 --body hi",
        ]
    );
}

#[test]
fn cli_forge_reads_pr_comments_for_a_marker() {
    let gh = FakeGh::new(false);
    gh.write(
        "view.json",
        r#"{"comments":[{"body":"x <!-- m -->"},{"author":"bot"}]}"#,
    );
    assert!(gh.forge.has_comment(&gh.dir.0, 7, "<!-- m -->").unwrap());
    assert!(!gh.forge.has_comment(&gh.dir.0, 7, "<!-- n -->").unwrap());
    gh.write("view.json", "not json");
    assert!(gh
        .forge
        .has_comment(&gh.dir.0, 7, "m")
        .unwrap_err()
        .starts_with("gh pr view"));
}

#[test]
fn cli_forge_reports_gh_failures_and_unreadable_answers() {
    let failing = FakeGh::new(true);
    let error = failing
        .forge
        .open_pull_request(&failing.dir.0, "wtm/x")
        .unwrap_err();
    assert_eq!(error, "gh pr list: gh: not logged in");

    let gh = FakeGh::new(false);
    let worktree = &gh.dir.0;
    gh.write("list.json", "{");
    assert!(gh.forge.open_pull_request(worktree, "b").is_err());
    gh.write("list.json", r#"[{"url":"u","title":"t"}]"#);
    assert_eq!(
        gh.forge.open_pull_request(worktree, "b").unwrap_err(),
        "gh answered a PR without number"
    );
    gh.write("list.json", r#"[{"number":1,"title":"t"}]"#);
    assert_eq!(
        gh.forge.open_pull_request(worktree, "b").unwrap_err(),
        "gh answered a PR without url"
    );
    gh.write("list.json", "[]");
    gh.write("created.json", "[]");
    let new = NewPullRequest {
        branch: "b".into(),
        title: "T".into(),
        body: String::new(),
        draft: false,
        assignee: "@me".into(),
    };
    assert_eq!(
        gh.forge.create_pull_request(worktree, &new).unwrap_err(),
        "gh pr create: no open PR for b afterwards"
    );
}

#[test]
fn failure_output_keeps_its_tail_on_a_character_boundary() {
    let output = format!("{}a", "é".repeat(3000));
    let kept = tail(&output);
    assert!(kept.len() < 4 * 1024 && kept.ends_with("éa"));
}
