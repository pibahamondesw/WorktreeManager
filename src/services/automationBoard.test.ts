import { describe, expect, it } from "vitest";
import {
  activeTime,
  boardCard,
  boardColumns,
  currentPace,
  effectiveLimits,
  isCampaign,
  paceLabel,
} from "./automationBoard";
import {
  AutomationDefinition,
  AutomationState,
  PipelineStep,
  RunItem,
  StepAttempt,
} from "./automations";
import { Task } from "../types";

const agentStep = (id: string, label: string): PipelineStep => ({
  id,
  label,
  next: null,
  onFail: "block",
  kind: {
    agent: {
      agent: { id: "claude", model: "opus", effort: null },
      skill: null,
      prompt: "Fix it",
      onReject: null,
    },
  },
});

const STEPS: PipelineStep[] = [
  agentStep("implement", "implementing"),
  {
    id: "verify",
    label: "verifying",
    next: null,
    onFail: "block",
    kind: { command: { command: ["make", "check"], repo: "app" } },
  },
  {
    id: "review",
    label: "awaiting review",
    next: null,
    onFail: "block",
    kind: { wait: { signal: "merged" } },
  },
];

const attempt = (step: string, overrides: Partial<StepAttempt> = {}): StepAttempt => ({
  step,
  attempt: 1,
  status: "succeeded",
  process_end: null,
  exit_code: null,
  outcome: null,
  stage: null,
  verdict: null,
  cause: null,
  head: null,
  session_id: null,
  conversation_id: null,
  ...overrides,
});

const item = (id: string, overrides: Partial<RunItem> = {}): RunItem => ({
  id,
  version: "1",
  title: null,
  status: "queued",
  phase: null,
  steps: [],
  run_id: "r1",
  test: false,
  ...overrides,
});

const TASK: Task = {
  id: "t-a",
  workspaceId: "w1",
  branchName: "pedro/wor-9-fix-cops",
  linearIssueIdentifier: "WOR-9",
  members: [],
  createdAt: "2026-11-01T00:00:00Z",
};

const definition = (overrides: Partial<AutomationDefinition> = {}): AutomationDefinition => ({
  name: "rubocop",
  description: null,
  trigger: { manual: true, schedule: null },
  steps: STEPS,
  selector: { maxNewItems: 2 },
  pools: { active: 1, openPrs: 3 },
  ...overrides,
});

describe("boardColumns", () => {
  it("groups items by phase: queued, the step they are on, then where they rest or end", () => {
    const items = [
      item("q"),
      item("i", { status: "active", phase: "implement" }),
      item("w", { status: "waiting", phase: "review" }),
      item("b", { status: "blocked", phase: "verify", cause: "budget:repair_rounds" }),
      item("d", { status: "deferred", phase: "implement" }),
      item("c", { status: "completed", phase: "review" }),
      item("f", { status: "failed", phase: "verify" }),
    ];

    const columns = boardColumns(STEPS, items, [], 0);

    expect(columns.map(({ label, cards }) => [label, cards.map((card) => card.item.id)])).toEqual([
      ["Queued", ["q"]],
      ["implementing", ["i"]],
      ["verifying", []],
      ["awaiting review", ["w"]],
      ["Blocked", ["b"]],
      ["Deferred", ["d"]],
      ["Integrated", ["c"]],
      ["Failed", ["f"]],
    ]);
  });

  it("keeps items on a step the definition no longer has, and open items without a phase queued", () => {
    const items = [
      item("gone", { status: "active", phase: "lint" }),
      item("starting", { status: "active", phase: null }),
      item("paused", { status: "paused", phase: "implement" }),
    ];

    const columns = boardColumns(STEPS, items, [], 0);

    expect(columns.map((column) => column.id)).toEqual([
      "queued",
      "step:implement",
      "step:verify",
      "step:review",
      "step:lint",
      "paused",
      "blocked",
      "deferred",
      "completed",
    ]);
    expect(columns[0].cards.map((card) => card.item.id)).toEqual(["starting"]);
    expect(columns[4].label).toBe("lint");
  });
});

describe("boardCard", () => {
  it("shows the issue, PR and branch of the item's task and what it did", () => {
    const blocked = item("a", {
      status: "blocked",
      phase: "verify",
      cause: "budget:repair_rounds",
      task_id: "t-a",
      counters: { "budget:repair_rounds": 2, "retries:verify": 1 },
      steps: [
        attempt("implement", { session_id: "s1", conversation_id: "c1" }),
        attempt("ship", {
          effects: [
            {
              effect: "github.pr_create",
              status: "staged",
              detail: null,
              reference: "https://github.com/o/r/pull/1",
              warnings: [],
            },
            {
              effect: "github.pr_create",
              status: "executed",
              detail: null,
              reference: "https://github.com/o/r/pull/7",
              warnings: [],
            },
          ],
        }),
        attempt("verify", {
          attempt: 3,
          status: "failed",
          verdict: "blocked",
          cause: "rubocop exited 1",
          reply: "app/models/user.rb:3 Style/Foo",
        }),
      ],
    });

    const card = boardCard(STEPS, blocked, [TASK], 0);

    expect(card).toMatchObject({
      key: "a",
      issue: "WOR-9",
      pr: "https://github.com/o/r/pull/7",
      branch: "pedro/wor-9-fix-cops",
      repairRounds: 3,
      reason: "budget:repair_rounds",
      evidence: "verifying #3 (failed · blocked): rubocop exited 1\napp/models/user.rb:3 Style/Foo",
      target: { taskId: "t-a", agent: "claude", conversationId: "c1" },
    });
  });

  it("opens the task of a running item without resuming its conversation", () => {
    const running = item("a", {
      status: "active",
      phase: "implement",
      task_id: "t-gone",
      test: true,
      steps: [
        attempt("ship", {
          effects: [
            {
              effect: "linear.upsert",
              status: "executed",
              detail: null,
              reference: "WOR-12",
              warnings: [],
            },
          ],
        }),
        attempt("implement", { status: "running", session_id: "s1", conversation_id: "c1" }),
      ],
    });

    const card = boardCard(STEPS, running, [TASK], 0);

    expect(card).toMatchObject({
      key: "test:a",
      issue: "WOR-12",
      branch: null,
      reason: null,
      evidence: null,
      target: { taskId: "t-gone", agent: "claude", conversationId: null },
      taskRemoved: true,
    });
    expect(boardCard(STEPS, item("q"), [], 0).taskRemoved).toBe(false);
    expect(boardCard(STEPS, item("q"), [], 0).target).toBeNull();
    expect(boardCard(STEPS, item("b", { status: "blocked", task_id: "t-a" }), [], 0)).toMatchObject(
      { evidence: null, target: { agent: null, conversationId: null } }
    );
  });
});

describe("activeTime", () => {
  it("counts agent and command attempts, and the open one while the item runs", () => {
    const steps = [
      attempt("implement", { elapsed_ms: 60_000 }),
      attempt("review", { elapsed_ms: 900_000 }),
      attempt("verify", { status: "running", elapsed_ms: 1_000, opened_at_ms: 10_000 }),
    ];

    expect(activeTime(STEPS, item("a", { status: "active", steps }), 40_000)).toBe(91_000);
    expect(activeTime(STEPS, item("a", { status: "blocked", steps }), 40_000)).toBe(61_000);
    expect(activeTime(STEPS, item("a", { steps: [attempt("implement")] }), 0)).toBe(0);
  });
});

describe("operator settings", () => {
  const state = (overrides: AutomationState["overrides"]): AutomationState => ({
    enabled: true,
    pause_reason: null,
    next_run_at: null,
    consecutive_failures: 0,
    overrides,
  });

  it("shows the definition's limits replaced by the operator's overrides", () => {
    const override = { value: 4, by: "cli" as const, at: "2026-11-04T10:00:00Z" };

    expect(effectiveLimits(definition(), state({ max_active: override }))).toEqual([
      { limit: "max_active", label: "Max active", value: 4, override },
      { limit: "max_open_prs", label: "Max open PRs", value: 3, override: null },
      { limit: "batch_size", label: "Batch size", value: 2, override: null },
    ]);
    expect(
      effectiveLimits(definition({ selector: { maxNewItems: null }, pools: undefined }), null).map(
        ({ value }) => value
      )
    ).toEqual([null, null, null]);
  });

  it("names the pace: continuous unless Next granted items", () => {
    expect(paceLabel(currentPace(null))).toBe("Continuous");
    const next = (remaining: number) =>
      state({
        pace: { value: { mode: "next", remaining }, by: "user", at: "2026-11-04T10:00:00Z" },
      });
    expect(paceLabel(currentPace(next(1)))).toBe("Next · 1 item granted");
    expect(paceLabel(currentPace(next(0)))).toBe("Next · 0 items granted");
  });

  it("only campaigns have a selector", () => {
    expect(isCampaign(definition())).toBe(true);
    expect(isCampaign(definition({ selector: null }))).toBe(false);
    expect(isCampaign(null)).toBe(false);
  });
});
