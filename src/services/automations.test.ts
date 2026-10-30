import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  AutomationEntry,
  AutomationRun,
  AutomationState,
  AutomationSummary,
  automationStatus,
  loadAutomationSummaries,
  pauseReasonLabel,
  runDuration,
  runResult,
  runStartedAt,
  triggerSummary,
} from "./automations";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const entry: AutomationEntry = {
  name: "pr-babysitter",
  path: "/home/.worktreemanager/automations/pr-babysitter/AUTOMATION.md",
  definition: {
    name: "pr-babysitter",
    description: null,
    trigger: { manual: true, schedule: null },
  },
  errors: [],
  warnings: [],
};
const enabled: AutomationState = {
  enabled: true,
  pause_reason: null,
  next_run_at: null,
  consecutive_failures: 0,
};
const run = (overrides: Partial<AutomationRun> = {}): AutomationRun => ({
  id: "1761800000000-abcdef12",
  trigger: { source: "manual", scheduled_for: null, test: false },
  status: "completed",
  skip_reason: null,
  failure_reason: null,
  outcome: "noop",
  claimed_at_ms: 1_000,
  ended_at_ms: 2_000,
  ...overrides,
});
const summary = (overrides: Partial<AutomationSummary> = {}): AutomationSummary => ({
  entry,
  state: enabled,
  stateError: null,
  latestRun: null,
  ...overrides,
});

describe("automationStatus", () => {
  it("derives every status in priority order", () => {
    const issue = { path: entry.path, field: "trigger", kind: "invalid", message: "bad" };
    expect(
      automationStatus(summary({ entry: { ...entry, definition: null, errors: [issue] } }))
    ).toBe("invalid");
    expect(automationStatus(summary({ latestRun: run({ status: "interrupted" }) }))).toBe(
      "running"
    );
    expect(automationStatus(summary({ state: { ...enabled, enabled: false } }))).toBe("paused");
    expect(automationStatus(summary({ state: null }))).toBe("paused");
    expect(automationStatus(summary({ state: { ...enabled, consecutive_failures: 2 } }))).toBe(
      "degraded"
    );
    expect(automationStatus(summary({ latestRun: run() }))).toBe("active");
  });
});

it("explains why an automation is paused", () => {
  expect(pauseReasonLabel(enabled)).toBeNull();
  expect(pauseReasonLabel(null)).toBeNull();
  expect(pauseReasonLabel({ ...enabled, enabled: false, pause_reason: "auto" })).toBe(
    "Paused after repeated failures"
  );
  expect(pauseReasonLabel({ ...enabled, enabled: false })).toBe("Paused");
});

it("summarizes triggers", () => {
  const schedule = { cron: "*/15 * * * *", tz: "UTC", catchUp: "latest" as const };
  const definition = entry.definition!;
  expect(triggerSummary(definition)).toBe("Manual only");
  expect(triggerSummary({ ...definition, trigger: { manual: false, schedule: null } })).toBe(
    "No trigger"
  );
  expect(triggerSummary({ ...definition, trigger: { manual: true, schedule } })).toBe(
    "Schedule */15 * * * * (UTC) · manual"
  );
  expect(triggerSummary({ ...definition, trigger: { manual: false, schedule } })).toBe(
    "Schedule */15 * * * * (UTC)"
  );
});

it("describes run results, start and duration", () => {
  expect(runResult(run({ outcome: "needs_attention" }))).toBe("Completed: needs attention");
  expect(runResult(run({ outcome: null }))).toBe("Completed");
  expect(runResult(run({ status: "skipped", skip_reason: "already_running" }))).toBe(
    "Skipped: already running"
  );
  expect(runResult(run({ status: "skipped" }))).toBe("Skipped");
  expect(runResult(run({ status: "failed", failure_reason: "step_failed" }))).toBe(
    "Failed: step failed"
  );
  expect(runResult(run({ status: "failed" }))).toBe("Failed");
  expect(runResult(run({ status: "running" }))).toBe("Running");

  expect(runStartedAt(run()).getTime()).toBe(1_000);
  expect(runStartedAt(run({ claimed_at_ms: null })).getTime()).toBe(1761800000000);

  expect(runDuration(run({ ended_at_ms: null }))).toBeNull();
  expect(runDuration(run({ ended_at_ms: 1_200 }))).toBe("<1s");
  expect(runDuration(run())).toBe("1s");
  expect(runDuration(run({ ended_at_ms: 1_000 + 125_000 }))).toBe("2m 5s");
  expect(runDuration(run({ ended_at_ms: 1_000 + 3_780_000 }))).toBe("1h 3m");
});

describe("loadAutomationSummaries", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("joins each definition with its state and latest run, keeping partial failures", async () => {
    const broken = { ...entry, name: "broken" };
    vi.mocked(invoke).mockImplementation((command, args) => {
      const name = (args as { name?: string } | undefined)?.name;
      if (command === "automations_list") return Promise.resolve([entry, broken]);
      if (command === "automation_state")
        return name === "broken" ? Promise.reject("state.json: bad") : Promise.resolve(enabled);
      if (command === "automation_runs_list")
        return name === "broken"
          ? Promise.reject("runs: bad")
          : Promise.resolve({ runs: [run()], next_before: null });
      return Promise.reject(new Error(`unexpected ${command}`));
    });

    const [ok, failed] = await loadAutomationSummaries();
    expect(ok).toEqual({ entry, state: enabled, stateError: null, latestRun: run() });
    expect(failed).toEqual({
      entry: broken,
      state: null,
      stateError: "state.json: bad",
      latestRun: null,
    });
    expect(invoke).toHaveBeenCalledWith("automation_runs_list", {
      name: "pr-babysitter",
      before: null,
      limit: 1,
    });
  });
});
