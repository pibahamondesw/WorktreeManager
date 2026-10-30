import { describe, expect, it } from "vitest";
import { AutomationItem, PipelineStep, StepAttempt, StepKind, Transition } from "./automations";
import {
  onReject,
  pipelineLayout,
  stepKindName,
  stepProgress,
  stepSummary,
  transitionLabel,
  waitConditionLabel,
} from "./automationPipeline";

const agent = (onReject: Transition | null = null, skill: string | null = null): StepKind => ({
  agent: { agent: { id: "claude", model: "opus", effort: null }, skill, prompt: null, onReject },
});
const command = (...argv: string[]): StepKind => ({ command: { command: argv, repo: "app" } });

const step = (
  id: string,
  kind: StepKind,
  overrides: Partial<Omit<PipelineStep, "id" | "kind">> = {}
): PipelineStep => ({ id, label: id, next: null, onFail: "fail", kind, ...overrides });

const goto = (target: string, max: { count: number } | { budget: string } | null = null) => ({
  goto: { step: target, max },
});

/** The PR babysitter campaign of docs/automations.md. */
const CAMPAIGN = [
  step("implement", agent(null, "implement")),
  step("verify", command("scripts/verify"), {
    onFail: goto("repair", { budget: "repair_rounds" }),
  }),
  step("review", agent(goto("repair", { budget: "repair_rounds" }))),
  step("publish", command("scripts/publish")),
  step("checks", { wait: { signal: "ci" } }, { onFail: goto("repair", { count: 5 }) }),
  step("repair", agent(), { next: "verify" }),
  step("approve", {
    approval: { title: "Ship it?", detail: null, onReject: "block" },
  }),
];

const attempt = (stepId: string, number: number, status: StepAttempt["status"]): StepAttempt => ({
  step: stepId,
  attempt: number,
  status,
  process_end: null,
  exit_code: null,
  outcome: null,
  stage: null,
  verdict: null,
  cause: null,
  head: null,
});

const item = (overrides: Partial<AutomationItem> = {}): AutomationItem => ({
  id: "pr-1",
  title: "PR #1",
  status: "active",
  phase: null,
  steps: [],
  ...overrides,
});

describe("pipelineLayout", () => {
  it("has no jumps for a linear routine", () => {
    expect(pipelineLayout([step("main", agent())])).toEqual({ edges: [], lanes: 0 });
    expect(
      pipelineLayout([
        step("a", agent()),
        step("b", command("make"), { next: "c" }),
        step("c", agent()),
      ])
    ).toEqual({ edges: [], lanes: 0 });
  });

  it("draws gotos and explicit next jumps in lanes that never overlap", () => {
    const { edges, lanes } = pipelineLayout(CAMPAIGN);
    expect(edges).toEqual([
      { from: 4, to: 5, kind: "on_fail", limit: "max 5", lane: 0 },
      { from: 2, to: 5, kind: "on_reject", limit: "budget repair_rounds", lane: 1 },
      { from: 1, to: 5, kind: "on_fail", limit: "budget repair_rounds", lane: 2 },
      { from: 5, to: 1, kind: "next", limit: null, lane: 3 },
    ]);
    expect(lanes).toBe(4);
  });

  it("reuses a lane for disjoint jumps and ignores unknown targets", () => {
    const { edges, lanes } = pipelineLayout([
      step("a", agent(), { onFail: goto("b") }),
      step("b", agent()),
      step("c", agent(), { onFail: goto("d") }),
      step("d", agent(), { onFail: goto("missing") }),
    ]);
    expect(edges.map(({ from, to, lane }) => [from, to, lane])).toEqual([
      [0, 1, 0],
      [2, 3, 0],
    ]);
    expect(lanes).toBe(1);
  });
});

describe("stepProgress", () => {
  it("marks every step not started without an item", () => {
    const progress = stepProgress(CAMPAIGN, null);
    expect(progress.get("implement")).toEqual({
      state: "not_started",
      attempts: 0,
      latest: null,
      current: false,
    });
  });

  it("takes the newest attempt of each step and the item's current phase", () => {
    const progress = stepProgress(
      CAMPAIGN,
      item({
        phase: "verify",
        steps: [
          attempt("implement", 1, "succeeded"),
          attempt("verify", 2, "running"),
          attempt("verify", 1, "failed"),
          attempt("repair", 1, "succeeded"),
        ],
      })
    );
    expect(progress.get("implement")).toMatchObject({ state: "succeeded", attempts: 1 });
    expect(progress.get("verify")).toMatchObject({ state: "running", attempts: 2, current: true });
    expect(progress.get("verify")?.latest?.attempt).toBe(2);
    expect(progress.get("repair")).toMatchObject({ state: "succeeded", current: false });
    expect(progress.get("review")?.state).toBe("not_started");
  });

  it("has no current step once the item ends", () => {
    const progress = stepProgress(
      CAMPAIGN,
      item({ status: "failed", phase: "verify", steps: [attempt("verify", 1, "failed")] })
    );
    expect(progress.get("verify")).toMatchObject({ state: "failed", current: false });
  });
});

describe("labels", () => {
  it("describes transitions, waits and steps", () => {
    expect(transitionLabel("block")).toBe("block");
    expect(transitionLabel(goto("repair", { count: 3 }))).toBe("goto repair (max 3)");
    expect(transitionLabel(goto("repair"))).toBe("goto repair");
    expect(waitConditionLabel({ duration_secs: 90 })).toBe("For 1m 30s");
    expect(waitConditionLabel({ until: "2026-11-01T00:00:00Z" })).toBe(
      "Until 2026-11-01T00:00:00Z"
    );
    expect(waitConditionLabel({ signal: "ci" })).toBe("Signal ci");

    expect(CAMPAIGN.map(stepKindName)).toEqual([
      "agent",
      "command",
      "agent",
      "command",
      "wait",
      "agent",
      "approval",
    ]);
    expect(CAMPAIGN.map(stepSummary)).toEqual([
      "claude · implement",
      "scripts/verify",
      "claude",
      "scripts/publish",
      "Signal ci",
      "claude",
      "Ship it?",
    ]);
    const publish = { ...CAMPAIGN[0], kind: { publish: ["git.push"] } as unknown as StepKind };
    expect(stepSummary(publish)).toBe("publish");
    expect(onReject(CAMPAIGN[6])).toBe("block");
    expect(onReject(CAMPAIGN[1])).toBeNull();
  });
});
