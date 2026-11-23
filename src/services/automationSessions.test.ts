import { beforeEach, expect, it, vi } from "vitest";
import type { InvokeArgs } from "@tauri-apps/api/core";
import { AutomationEntry, AutomationRun, PipelineStep, StepAttempt } from "./automations";

const invoke = vi.hoisted(() => vi.fn<(command: string, args?: InvokeArgs) => Promise<unknown>>());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { findSessionSources, loadSessionSources, sessionTitle } from "./automationSessions";

const STEPS: PipelineStep[] = [
  {
    id: "review",
    label: "Review the diff",
    next: null,
    onFail: "fail",
    kind: { command: { command: ["true"], repo: "app" } },
  },
];

const entry = (name: string): AutomationEntry => ({
  name,
  path: `/automations/${name}`,
  definition: { name, description: null, trigger: { manual: true, schedule: null }, steps: STEPS },
  errors: [],
  warnings: [],
});

const attempt = (step: string, sessionId: string | null): StepAttempt => ({
  step,
  attempt: 1,
  status: "running",
  process_end: null,
  exit_code: null,
  outcome: null,
  stage: null,
  verdict: null,
  cause: null,
  head: null,
  session_id: sessionId,
  conversation_id: null,
});

const run = (id: string, attempts: StepAttempt[]): AutomationRun => ({
  id,
  trigger: null,
  status: "running",
  skip_reason: null,
  failure_reason: null,
  outcome: null,
  claimed_at_ms: null,
  ended_at_ms: null,
  items: [{ id: `item-${id}`, title: null, status: "active", phase: "review", steps: attempts }],
});

beforeEach(() => {
  invoke.mockReset();
});

it("finds the newest attempt of each session asked for, across runs", () => {
  const sources = findSessionSources(
    [
      {
        entry: entry("nightly"),
        runs: [
          run("r2", [attempt("fix", "s1")]),
          run("r1", [attempt("review", "s1"), attempt("review", "s2"), attempt("review", null)]),
        ],
      },
    ],
    ["s1", "s2", "missing"]
  );
  expect(Object.keys(sources).sort()).toEqual(["s1", "s2"]);
  expect(sources.s1).toMatchObject({ automation: "nightly", attempt: { step: "fix" } });
  expect(sources.s1.run.id).toBe("r2");
  expect(sources.s2.item.id).toBe("item-r1");
});

it("names a session after its automation and step, by id when the step is gone", () => {
  const [found] = Object.values(
    findSessionSources(
      [{ entry: entry("nightly"), runs: [run("r1", [attempt("review", "s1")])] }],
      ["s1"]
    )
  );
  expect(sessionTitle(found)).toBe("nightly · Review the diff");
  expect(sessionTitle({ ...found, attempt: attempt("removed", "s1") })).toBe("nightly · removed");
  expect(sessionTitle({ ...found, steps: [] })).toBe("nightly · review");
});

it("searches the newest runs of every automation and skips one whose runs fail to load", async () => {
  invoke.mockImplementation((command, args) => {
    if (command === "automations_list") return Promise.resolve([entry("broken"), entry("nightly")]);
    const { name } = args as { name: string };
    return name === "broken"
      ? Promise.reject("unreadable log")
      : Promise.resolve({ runs: [run("r1", [attempt("review", "s1")])], next_before: null });
  });
  const sources = await loadSessionSources(["s1"]);
  expect(sources.s1.automation).toBe("nightly");
  expect(invoke).toHaveBeenCalledWith("automation_runs_list", {
    name: "nightly",
    before: null,
    limit: 10,
  });
});
