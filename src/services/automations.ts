import { invoke } from "@tauri-apps/api/core";

export interface DefinitionIssue {
  path: string;
  field: string;
  kind: string;
  message: string;
}

interface AutomationDefinition {
  name: string;
  description: string | null;
  trigger: {
    manual: boolean;
    schedule: { cron: string; tz: string; catchUp: "latest" | "skip" } | null;
  };
}

export interface AutomationEntry {
  name: string;
  path: string;
  /** Null exactly when `errors` is not empty. */
  definition: AutomationDefinition | null;
  errors: DefinitionIssue[];
  warnings: DefinitionIssue[];
}

type PauseReason = "new" | "manual" | "auto" | "completed" | "stop_after";

/** The run store's `state.json`, serialized in snake_case. */
export interface AutomationState {
  enabled: boolean;
  pause_reason: PauseReason | null;
  next_run_at: string | null;
  consecutive_failures: number;
}

type RunStatus =
  | "skipped"
  | "starting"
  | "running"
  | "interrupted"
  | "completed"
  | "failed"
  | "canceled";

export interface AutomationRun {
  id: string;
  trigger: { source: string; scheduled_for: string | null; test: boolean } | null;
  status: RunStatus;
  skip_reason: string | null;
  failure_reason: string | null;
  outcome: string | null;
  claimed_at_ms: number | null;
  ended_at_ms: number | null;
}

export interface AutomationRunPage {
  runs: AutomationRun[];
  next_before: string | null;
}

export interface AutomationSummary {
  entry: AutomationEntry;
  state: AutomationState | null;
  stateError: string | null;
  latestRun: AutomationRun | null;
}

export type AutomationStatus = "invalid" | "running" | "paused" | "degraded" | "active";

export const listAutomationRuns = (name: string, before: string | null, limit: number) =>
  invoke<AutomationRunPage>("automation_runs_list", { name, before, limit });

export async function loadAutomationSummaries(): Promise<AutomationSummary[]> {
  const entries = await invoke<AutomationEntry[]>("automations_list");
  return Promise.all(entries.map(summarize));
}

async function summarize(entry: AutomationEntry): Promise<AutomationSummary> {
  const [state, runs] = await Promise.allSettled([
    invoke<AutomationState>("automation_state", { name: entry.name }),
    listAutomationRuns(entry.name, null, 1),
  ]);
  return {
    entry,
    state: state.status === "fulfilled" ? state.value : null,
    stateError: state.status === "rejected" ? String(state.reason) : null,
    latestRun: runs.status === "fulfilled" ? (runs.value.runs[0] ?? null) : null,
  };
}

const ACTIVE_RUN_STATUSES: RunStatus[] = ["starting", "running", "interrupted"];

export function automationStatus({ entry, state, latestRun }: AutomationSummary): AutomationStatus {
  if (entry.errors.length > 0) return "invalid";
  if (latestRun && ACTIVE_RUN_STATUSES.includes(latestRun.status)) return "running";
  if (!state || !state.enabled) return "paused";
  return state.consecutive_failures > 0 ? "degraded" : "active";
}

export const AUTOMATION_STATUS_LABELS: Record<AutomationStatus, string> = {
  invalid: "Invalid",
  running: "Running",
  paused: "Paused",
  degraded: "Degraded",
  active: "Active",
};

const PAUSE_REASON_LABELS: Record<PauseReason, string> = {
  new: "New automation, not enabled yet",
  manual: "Paused by you",
  auto: "Paused after repeated failures",
  completed: "Completed its runs",
  stop_after: "Reached its stop_after limit",
};

export function pauseReasonLabel(state: AutomationState | null): string | null {
  if (!state || state.enabled) return null;
  return state.pause_reason ? PAUSE_REASON_LABELS[state.pause_reason] : "Paused";
}

export function triggerSummary(definition: AutomationDefinition): string {
  const { manual, schedule } = definition.trigger;
  if (!schedule) return manual ? "Manual only" : "No trigger";
  const scheduled = `Schedule ${schedule.cron} (${schedule.tz})`;
  return manual ? `${scheduled} · manual` : scheduled;
}

const humanize = (value: string) => value.replace(/_/g, " ");

export function runResult(run: AutomationRun): string {
  switch (run.status) {
    case "skipped":
      return run.skip_reason ? `Skipped: ${humanize(run.skip_reason)}` : "Skipped";
    case "failed":
      return run.failure_reason ? `Failed: ${humanize(run.failure_reason)}` : "Failed";
    case "completed":
      return run.outcome ? `Completed: ${humanize(run.outcome)}` : "Completed";
    default:
      return run.status.charAt(0).toUpperCase() + run.status.slice(1);
  }
}

/** Run ids start with the 13-digit claim time, used when the claim event is missing. */
export function runStartedAt(run: AutomationRun): Date {
  return new Date(run.claimed_at_ms ?? Number(run.id.slice(0, 13)));
}

export function runDuration(run: AutomationRun): string | null {
  if (run.claimed_at_ms === null || run.ended_at_ms === null) return null;
  return formatDuration(run.ended_at_ms - run.claimed_at_ms);
}

function formatDuration(ms: number): string {
  const seconds = Math.round(ms / 1000);
  if (seconds < 1) return "<1s";
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}
