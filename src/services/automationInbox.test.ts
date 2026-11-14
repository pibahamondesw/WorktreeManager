import { describe, expect, it } from "vitest";
import {
  EMPTY_INBOX_MARKS,
  InboxItem,
  badgeCount,
  approvalAnswer,
  conversationTarget,
  editedParams,
  opensInInbox,
  freshEntries,
  inboxEntries,
  mark,
  takenOverTaskIds,
  unreadCount,
  withMarks,
} from "./automationInbox";
import {
  AutomationEntry,
  AutomationRun,
  PermissionRequest,
  ReportPolicy,
  StepAttempt,
} from "./automations";

const STEPS = [
  {
    id: "triage",
    label: "triage",
    next: null,
    onFail: "fail" as const,
    kind: {
      agent: {
        agent: { id: "claude", model: "opus", effort: null },
        skill: null,
        prompt: null,
        onReject: null,
      },
    },
  },
];

function entry(name = "babysitter", report?: ReportPolicy): AutomationEntry {
  return {
    name,
    path: `/automations/${name}/AUTOMATION.md`,
    definition: {
      name,
      description: null,
      trigger: { manual: true, schedule: null },
      steps: STEPS,
      report,
    },
    errors: [],
    warnings: [],
  };
}

function attempt(overrides: Partial<StepAttempt> = {}): StepAttempt {
  return {
    step: "triage",
    attempt: 1,
    status: "succeeded",
    process_end: "turn_completed",
    exit_code: null,
    outcome: "success",
    stage: null,
    verdict: null,
    cause: null,
    head: null,
    session_id: "s1",
    conversation_id: "c1",
    ...overrides,
  };
}

function run(id: string, overrides: Partial<AutomationRun> = {}): AutomationRun {
  return {
    id,
    trigger: { source: "schedule", scheduled_for: null, test: false },
    status: "completed",
    skip_reason: null,
    failure_reason: null,
    outcome: "reported",
    claimed_at_ms: 1000,
    ended_at_ms: Number(id),
    items: [{ id: "routine", title: null, status: "completed", phase: null, steps: [attempt()] }],
    task_id: "t1",
    ...overrides,
  };
}

const question = (overrides: Partial<PermissionRequest> = {}): PermissionRequest => ({
  item: "routine",
  step: "triage",
  attempt: 1,
  tool: "Question",
  input: [{ question: "Which base branch?" }],
  resolution: "denied",
  request_id: "r1",
  session_id: "s1",
  kind: "question",
  ...overrides,
});

const keys = (entries: { key: string }[]) => entries.map((e) => e.key);

describe("inboxEntries", () => {
  it("reports findings, failures and blocked runs newest first, and skips other runs", () => {
    const runs = [
      run("5", { status: "running", outcome: null }),
      run("4", { status: "skipped", outcome: null }),
      run("3", { outcome: "needs_attention" }),
      run("2", { status: "failed", outcome: null, failure_reason: "deadline" }),
      run("1"),
    ];
    const entries = inboxEntries([{ entry: entry(), runs }]);
    expect(entries.map(({ key, kind, title }) => [key, kind, title])).toEqual([
      ["run:babysitter:3", "needs_attention", "Needs attention"],
      ["run:babysitter:2", "failed", "Failed: deadline"],
      ["run:babysitter:1", "findings", "Findings to review"],
    ]);
    expect(entries[2]).toMatchObject({
      automation: "babysitter",
      runId: "1",
      at: 1,
      test: false,
      target: { taskId: "t1", agent: "claude", conversationId: "c1" },
      autoArchived: false,
      notify: true,
    });
  });

  it("auto-archives noop runs unless archive_noop is off", () => {
    const runs = [run("1", { outcome: "noop" })];
    const [archived] = inboxEntries([{ entry: entry(), runs }]);
    expect(archived).toMatchObject({ kind: "noop", autoArchived: true, notify: false });
    const always = { inbox: "always" as const, archiveNoop: false, notify: true };
    expect(inboxEntries([{ entry: entry("a", always), runs }])[0]).toMatchObject({
      autoArchived: false,
      notify: true,
    });
    const quiet = { inbox: "on_findings" as const, archiveNoop: false, notify: true };
    expect(inboxEntries([{ entry: entry("a", quiet), runs }])).toEqual([]);
  });

  it("follows the report policy for completed runs", () => {
    const runs = [run("2", { outcome: "dispatched" }), run("1")];
    const policy = (inbox: ReportPolicy["inbox"], notify = true) => ({
      inbox,
      archiveNoop: true,
      notify,
    });
    const kinds = (report: ReportPolicy) =>
      inboxEntries([{ entry: entry("a", report), runs }]).map((e) => e.kind);
    expect(kinds(policy("always"))).toEqual(["completed", "findings"]);
    expect(kinds(policy("on_findings"))).toEqual(["findings"]);
    expect(kinds(policy("on_failure"))).toEqual([]);
    expect(kinds(policy("never"))).toEqual([]);
    const silent = inboxEntries([{ entry: entry("a", policy("always", false)), runs }]);
    expect(silent.every((e) => !e.notify)).toBe(true);
  });

  it("describes a run by its result, reply, task error or failing step", () => {
    const detail = (overrides: Partial<AutomationRun>) =>
      inboxEntries([{ entry: entry(), runs: [run("1", overrides)] }])[0].detail;
    const items = (step: StepAttempt, cause: string | null = null) => [
      { id: "routine", title: null, status: "blocked", phase: null, cause, steps: [step] },
    ];
    const summary = { verdict: "pass", summary: "PR #12 is red", findings: [{}, {}] };
    expect(detail({ items: items(attempt({ result: summary })) })).toBe(
      "2 findings: PR #12 is red"
    );
    expect(detail({ items: items(attempt({ result: { findings: [{}] } })) })).toBe("1 finding");
    expect(detail({ items: items(attempt({ reply: "  All\n green " })) })).toBe("All green");
    expect(detail({ items: items(attempt(), "budget:repair_rounds") })).toBe(
      "budget:repair_rounds"
    );
    expect(detail({ items: items(attempt({ cause: "agent_exited" })) })).toBe(
      "triage: agent exited"
    );
    expect(detail({ task_error: { message: "Branch exists", worktrees: [] }, items: [] })).toBe(
      "Branch exists"
    );
    expect(detail({ items: items(attempt({ reply: "x".repeat(400) })) })).toHaveLength(280);
    expect(detail({ items: items(attempt()) })).toBeNull();
  });

  it("asks a question once however many runs ask it again", () => {
    const runs = [
      run("3", { outcome: "noop", permission_requests: [question({ session_id: "s3" })] }),
      run("2", { outcome: "noop", permission_requests: [question({ session_id: "s2" })] }),
      run("1", {
        outcome: "noop",
        permission_requests: [question({ input: [{ question: "Other?" }] })],
      }),
    ];
    const questions = inboxEntries([{ entry: entry(), runs }]).filter((e) => e.kind === "question");
    expect(questions.map(({ runId, title }) => [runId, title])).toEqual([
      ["2", "Which base branch?"],
      ["1", "Other?"],
    ]);
    expect(questions[0]).toMatchObject({ detail: "Unanswered in triage", notify: true });
  });

  it("lists each denied or defaulted permission by session and request", () => {
    const denied = question({
      kind: "permission",
      tool: "Bash",
      input: { command: "gh pr merge" },
    });
    const allowed = { ...denied, resolution: "allowed" as const, request_id: "r2", input: null };
    const legacy: PermissionRequest = {
      step: "triage",
      attempt: 1,
      tool: "WebFetch",
      input: "https://example.com",
      resolution: "denied",
    };
    const runs = [run("1", { outcome: "noop", permission_requests: [denied, allowed, legacy] })];
    const entries = inboxEntries([{ entry: entry(), runs }]).filter((e) => !e.autoArchived);
    expect(entries.map(({ key, title, detail }) => [key, title, detail])).toEqual([
      ["permission:s1:r1", "Denied: Bash", 'triage: {"command":"gh pr merge"}'],
      ["permission:s1:r2", "Allowed by default: Bash", "triage"],
      ["permission:babysitter:1:2", "Denied: WebFetch", "triage: https://example.com"],
    ]);
    expect(entries[0].target).toEqual({ taskId: "t1", agent: "claude", conversationId: "c1" });
  });

  it("uses the question's tool when its questions cannot be read", () => {
    const runs = [run("1", { outcome: "noop", permission_requests: [question({ input: "?" })] })];
    const [asked] = inboxEntries([{ entry: entry(), runs }]).filter((e) => e.kind === "question");
    expect(asked.title).toBe("Question");
  });

  it("falls back to the default report policy for invalid definitions", () => {
    const invalid = { ...entry(), definition: null };
    const runs = [run("2", { outcome: "noop" }), run("1")];
    const entries = inboxEntries([{ entry: invalid, runs }]);
    expect(entries.map((e) => [e.kind, e.autoArchived, e.target?.agent])).toEqual([
      ["noop", true, null],
      ["findings", false, null],
    ]);
  });
});

describe("conversationTarget", () => {
  it("resumes a conversation only once its run ended, and needs a task", () => {
    const ended = run("1");
    expect(conversationTarget(STEPS, ended, attempt())).toEqual({
      taskId: "t1",
      agent: "claude",
      conversationId: "c1",
    });
    expect(conversationTarget(STEPS, run("1", { status: "running" }), attempt())).toEqual({
      taskId: "t1",
      agent: "claude",
      conversationId: null,
    });
    expect(conversationTarget(STEPS, ended, undefined)).toEqual({
      taskId: "t1",
      agent: null,
      conversationId: null,
    });
    expect(conversationTarget(STEPS, run("1", { task_id: null }), attempt())).toBeNull();
  });
});

describe("marks", () => {
  const entries = () =>
    inboxEntries([{ entry: entry(), runs: [run("3", { outcome: "noop" }), run("2"), run("1")] }]);

  it("keeps read and archived state by key and counts what is still pending", () => {
    let marks = mark(EMPTY_INBOX_MARKS, "read", ["run:babysitter:2"], true);
    marks = mark(marks, "archived", ["run:babysitter:1"], true);
    const items = withMarks(entries(), marks);
    expect(items.map(({ key, read, archived }) => [key, read, archived])).toEqual([
      ["run:babysitter:3", false, true],
      ["run:babysitter:2", true, false],
      ["run:babysitter:1", false, true],
    ]);
    expect(unreadCount(items)).toBe(0);
    const unarchived = withMarks(entries(), mark(marks, "archived", ["run:babysitter:1"], false));
    expect(unreadCount(unarchived)).toBe(1);
    expect(badgeCount(unarchived)).toBe(1);
    const silenced = unarchived.map((item) => ({ ...item, notify: false }));
    expect(badgeCount(silenced)).toBe(0);
  });

  it("moves a re-marked key to the end and drops the oldest beyond the cap", () => {
    const many = Array.from({ length: 1001 }, (_, index) => `k${index}`);
    const marks = mark(mark(EMPTY_INBOX_MARKS, "read", many, true), "read", ["k1"], true);
    expect(marks.read).toHaveLength(1000);
    expect(marks.read[0]).toBe("k2");
    expect(marks.read.at(-1)).toBe("k1");
    expect(marks.archived).toEqual([]);
  });
});

describe("freshEntries", () => {
  const item = (key: string, overrides: Partial<InboxItem> = {}): InboxItem => ({
    ...inboxEntries([{ entry: entry(), runs: [run("1")] }])[0],
    key,
    read: false,
    archived: false,
    ...overrides,
  });

  it("notifies new unread entries once, never the history of an automation loaded first", () => {
    const known = new Map<string, Set<string>>();
    expect(freshEntries([item("old")], ["babysitter"], known)).toEqual([]);
    const next = [
      item("old"),
      item("new"),
      item("read", { read: true }),
      item("quiet", { notify: false }),
      item("other", { automation: "unloaded" }),
    ];
    expect(keys(freshEntries(next, ["babysitter"], known))).toEqual(["new"]);
    expect(freshEntries(next, ["babysitter"], known)).toEqual([]);
  });

  it("notifies the first entry of an automation that had none when first loaded", () => {
    const known = new Map<string, Set<string>>();
    expect(freshEntries([], ["babysitter"], known)).toEqual([]);
    expect(keys(freshEntries([item("first")], ["babysitter"], known))).toEqual(["first"]);
  });
});

describe("approval entries", () => {
  const ship = {
    id: "ship",
    label: "ship",
    next: null,
    onFail: "fail" as const,
    kind: {
      approval: {
        title: "Ship it?",
        detail: "Check the diff",
        params: { files: ["a.rb"] },
        default: "reject" as const,
        onReject: null,
      },
    },
  };
  const approving = (report?: ReportPolicy): AutomationEntry => {
    const base = entry("babysitter", report);
    return { ...base, definition: { ...base.definition!, steps: [...STEPS, ship] } };
  };
  const waiting = attempt({
    step: "ship",
    status: "waiting",
    outcome: null,
    session_id: null,
    conversation_id: null,
    wait: { timeout_at_ms: 5000, title: "Ship babysitter?" },
  });
  const pendingRun = (steps: StepAttempt[]) =>
    run("7", {
      status: "running",
      outcome: null,
      items: [{ id: "routine", title: null, status: "waiting", phase: "ship", steps }],
    });

  it("asks once per attempt and records the answer, even with inbox never", () => {
    const never = { inbox: "never" as const, archiveNoop: true, notify: true };
    const [pending] = inboxEntries([
      { entry: approving(never), runs: [pendingRun([attempt(), waiting])] },
    ]);
    expect(pending).toMatchObject({
      key: "approval:babysitter:routine:ship:7:1",
      kind: "approval",
      title: "Ship babysitter?",
      detail: "Check the diff",
      target: { taskId: "t1", agent: null, conversationId: null },
      approval: {
        runId: "7",
        item: "routine",
        step: "ship",
        attempt: 1,
        params: { files: ["a.rb"] },
        timeoutAt: 5000,
        default: "reject",
        answer: null,
      },
    });
    const rejected = {
      ...waiting,
      status: "rejected" as const,
      result: { decision: "reject", comment: "Split it" },
    };
    const [answered] = inboxEntries([{ entry: approving(), runs: [pendingRun([rejected])] }]);
    expect(answered.detail).toBe("Rejected: Split it");
    expect(answered.approval?.answer).toBe("Rejected: Split it");
    expect(inboxEntries([{ entry: approving(), runs: [pendingRun([attempt()])] }])).toEqual([]);
  });

  it("shows a campaign item's rendered detail and links the item's task", () => {
    const rendered = { ...waiting, wait: { title: "Ready: #7", detail: "Review #7" } };
    const campaignRun = run("7", {
      status: "running",
      outcome: null,
      task_id: null,
      items: [
        { id: "a", title: "A", status: "waiting", phase: "ship", steps: [rendered], task_id: "t9" },
      ],
    });
    const [pending] = inboxEntries([{ entry: approving(), runs: [campaignRun] }]);
    expect(pending).toMatchObject({
      title: "Ready: #7",
      detail: "Review #7",
      target: { taskId: "t9", agent: null, conversationId: null },
    });
  });

  it("opens approvals in the Inbox and other entries in their task", () => {
    const [pending] = inboxEntries([{ entry: approving(), runs: [pendingRun([waiting])] }]);
    expect(opensInInbox(pending)).toBe(true);
    const [findings] = inboxEntries([{ entry: entry(), runs: [run("1")] }]);
    expect(opensInInbox(findings)).toBe(false);
  });

  it("describes how each approval ended", () => {
    const ended = (overrides: Partial<StepAttempt>) =>
      approvalAnswer({ ...waiting, status: "succeeded", ...overrides });
    expect(approvalAnswer(waiting)).toBeNull();
    expect(ended({ signal: { decision: "approve" } })).toBe("Approved");
    expect(ended({ result: { decision: "approve", default: true } })).toBe(
      "Approved by default after the timeout"
    );
    expect(ended({ status: "failed", cause: "invalid_answer" })).toBe("invalid answer");
    expect(ended({ status: "canceled" })).toBe("canceled");
  });

  it("reads edited params back as their type and rejects invalid JSON", () => {
    const params = { files: ["a.rb"], base: "main", depth: 2 };
    expect(editedParams(params, { files: '["a.rb"]', base: "main" })).toEqual({ params: {} });
    expect(editedParams(params, { files: '["b.rb"]', base: "dev", depth: "3" })).toEqual({
      params: { files: ["b.rb"], base: "dev", depth: 3 },
    });
    expect(editedParams(params, { files: "b.rb" })).toEqual({
      error: 'files must be JSON, like ["a.rb"]',
    });
  });
});

describe("taken-over items", () => {
  const takenOver = {
    id: "routine",
    title: null,
    status: "paused",
    phase: "triage",
    cause: "human_takeover",
    steps: [attempt({ status: "canceled" })],
  };

  it("show while the owner holds them, without notifying or repeating the run's outcome", () => {
    const routine = run("2000", { outcome: "needs_attention", items: [takenOver] });
    const campaign = run("3000", {
      outcome: "dispatched",
      task_id: null,
      items: [{ ...takenOver, id: "unit-1", title: "Unit 1", task_id: "t9", steps: [] }],
    });

    const entries = inboxEntries([{ entry: entry(), runs: [routine, campaign] }]);

    expect(entries.map((item) => [item.key, item.kind, item.detail, item.notify])).toEqual([
      ["takeover:babysitter:3000:unit-1:triage:0", "taken_over", "Item Unit 1", false],
      [
        "takeover:babysitter:2000:routine:triage:1",
        "taken_over",
        "Hand it back from the Pipeline tab or the CLI",
        false,
      ],
    ]);
    expect(entries[0].target).toEqual({ taskId: "t9", agent: null, conversationId: null });
    expect(entries[1].title).toBe("Taken over: triage");
    expect(entries[1].target?.conversationId).toBe("c1");
    expect(takenOverTaskIds(entries)).toEqual(new Set(["t9", "t1"]));
  });

  it("let a reopened run report its new outcome as a new entry", () => {
    const reopened = run("2000", { reopened_at_ms: 1500 });
    expect(inboxEntries([{ entry: entry(), runs: [reopened] }])[0].key).toBe(
      "run:babysitter:2000:1500"
    );
    expect(takenOverTaskIds(inboxEntries([{ entry: entry(), runs: [run("2000")] }]))).toEqual(
      new Set()
    );
  });
});
