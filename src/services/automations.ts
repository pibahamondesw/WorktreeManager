import { invoke } from "@tauri-apps/api/core";

export interface DefinitionIssue {
  path: string;
  field: string;
  kind: string;
  message: string;
}

type InboxPolicy = "always" | "on_findings" | "on_failure" | "never";

/** The definition's `report`: when runs reach the Inbox and whether they notify. */
export interface ReportPolicy {
  inbox: InboxPolicy;
  archiveNoop: boolean;
  notify: boolean;
}

export interface TargetRepo {
  id: string;
  name: string;
}

export interface AutomationDefinition {
  name: string;
  description: string | null;
  trigger: {
    manual: boolean;
    schedule: { cron: string; tz: string; catchUp: "latest" | "skip" } | null;
    cli?: boolean;
  };
  steps: PipelineStep[];
  target?: { repos: TargetRepo[] };
  report?: ReportPolicy;
  /** Present in campaigns. */
  selector?: { maxNewItems: number | null } | null;
  pools?: { active: number; openPrs: number };
}

type GotoLimit = { count: number } | { budget: string };

export type Transition =
  | "fail"
  | "block"
  | "defer"
  | "continue"
  | { goto: { step: string; max: GotoLimit | null } };

export type WaitCondition =
  | { duration_secs: number }
  | { until: string }
  | { signal: string }
  | { event: { watcher: string; params: Record<string, unknown> } };

export type ApprovalDefault = "approve" | "reject";

export type StepKind =
  | {
      agent: {
        agent: { id: string; model: string; effort: string | null };
        skill: string | null;
        prompt: string | null;
        onReject: Transition | null;
      };
    }
  | { command: { command: string[]; repo: string } }
  | { wait: WaitCondition }
  | {
      approval: {
        title: string;
        detail: string | null;
        /** Values the approver may edit. */
        params?: Record<string, unknown>;
        default?: ApprovalDefault | null;
        onReject: Transition | null;
      };
    }
  | { publish: PublishEffect[] };

/** One effect of a publish step, named as in the definition, with its params. */
interface PublishEffect {
  effect: string;
  [param: string]: unknown;
}

type EffectStatus =
  | "executed"
  | "staged"
  | "reconciled"
  | "skipped"
  | "denied"
  | "capped"
  | "failed";

/** What one effect of a publish attempt did, would do in a Test run, or why not. */
export interface StepEffect {
  effect: string;
  status: EffectStatus;
  detail: string | null;
  reference: string | null;
  warnings: string[];
}

export interface PipelineStep {
  id: string;
  label: string;
  /** Null goes to the following step. */
  next: string | null;
  onFail: Transition;
  kind: StepKind;
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

type Actor = "user" | "cli" | "auto";

/** The latest pause or resume. */
interface Toggle {
  enabled: boolean;
  by: Actor;
  reason: string | null;
  at: string;
}

/** A value the operator set in place of the definition's. */
export interface Override<T> {
  value: T;
  by: Actor;
  at: string;
}

export type Pace = { mode: "continuous" } | { mode: "next"; remaining: number };

export type Limit = "max_active" | "max_open_prs" | "batch_size";

/** Operator overrides of a campaign; an absent field keeps the definition's value. */
type Overrides = Partial<Record<Limit, Override<number>>> & { pace?: Override<Pace> };

/** The run store's `state.json`, serialized in snake_case. */
export interface AutomationState {
  enabled: boolean;
  pause_reason: PauseReason | null;
  next_run_at: string | null;
  consecutive_failures: number;
  last_toggle?: Toggle | null;
  overrides?: Overrides;
}

export type StepStatus =
  | "pending"
  | "running"
  | "waiting"
  | "succeeded"
  | "failed"
  | "rejected"
  | "canceled"
  | "skipped";

export interface StepAttempt {
  step: string;
  attempt: number;
  status: StepStatus;
  process_end: string | null;
  exit_code: number | null;
  outcome: string | null;
  stage: string | null;
  verdict: string | null;
  cause: string | null;
  head: string | null;
  /** The headless session of an agent attempt. */
  session_id: string | null;
  /** The agent's conversation, which the task's chat can resume. */
  conversation_id: string | null;
  /** The agent's final reply, and the structured result read from it. */
  reply?: string | null;
  result?: unknown;
  /** The effects of a publish attempt, in order. */
  effects?: StepEffect[];
  /** When the attempt last opened, while it is open. */
  opened_at_ms?: number | null;
  elapsed_ms?: number;
  /** When a wait or approval attempt is due and times out, and the approval's title. */
  wait?: ArmedWait;
  /** The payload that resolved a signal wait, or an approval's answer. */
  signal?: unknown;
}

interface ArmedWait {
  wake_at_ms?: number;
  timeout_at_ms?: number;
  title?: string;
}

/** An approval's answer: edited params are available to later steps as `item.data`. */
export interface ApprovalAnswer {
  decision: ApprovalDefault;
  comment?: string;
  params?: Record<string, unknown>;
}

export interface AutomationItem {
  id: string;
  title: string | null;
  status: string;
  /** The step the item is on while it runs. */
  phase: string | null;
  /** Why a blocked or deferred item rests. */
  cause?: string | null;
  steps: StepAttempt[];
  /** `goto` rounds taken, by counter. */
  counters?: Record<string, number>;
  /** The item's own task, in campaigns. */
  task_id?: string | null;
}

/** An item on the board: the newest of its id, with the run that holds it. */
export interface RunItem extends AutomationItem {
  version: string;
  run_id: string;
  test: boolean;
}

/** A tool request the agent's unattended policy denied, or that no one answered in time. */
export interface PermissionRequest {
  /** Missing in runs recorded before the Inbox. */
  item?: string;
  step: string;
  attempt: number;
  tool: string;
  /** For questions, the questions asked. */
  input: unknown;
  resolution: "denied" | "allowed";
  request_id?: string | null;
  session_id?: string | null;
  kind?: "permission" | "question";
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
  items: AutomationItem[];
  task_id?: string | null;
  task_error?: { message: string; worktrees: string[] } | null;
  permission_requests?: PermissionRequest[];
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

export interface StartedRun {
  run_id: string;
  status: RunStatus;
  skip_reason: string | null;
}

/** Returns once the run is claimed; it runs in the background. */
export const startAutomationRun = (
  name: string,
  test: boolean,
  payload: Record<string, unknown> | null = null
) => invoke<StartedRun>("automation_run", { name, test, payload });

type Operator = Exclude<Actor, "auto">;

export const pauseAutomation = (name: string, by: Operator, reason: string | null = null) =>
  invoke<AutomationState>("automation_pause", { name, by, reason });

export const resumeAutomation = (name: string, by: Operator, reason: string | null = null) =>
  invoke<AutomationState>("automation_resume", { name, by, reason });

export const listAutomationItems = (name: string) =>
  invoke<RunItem[]>("automation_items_list", { name });

/** Grants one more item; the run is the selection pass it started, if manual runs are allowed. */
export const nextAutomationItem = (name: string, by: Operator) =>
  invoke<{ state: AutomationState; run: StartedRun | null }>("automation_next", { name, by });

export const runAutomationContinuously = (name: string, by: Operator) =>
  invoke<AutomationState>("automation_continuous", { name, by });

/** Null goes back to the definition's value. */
export const setAutomationLimit = (
  name: string,
  by: Operator,
  limit: Limit,
  value: number | null
) => invoke<AutomationState>("automation_set_limit", { name, by, limit, value });
interface StepSignal {
  name: string;
  runId: string;
  step: string;
  item: string;
  payload: Record<string, unknown>;
}

/** Sends a waiting signal or approval step its payload, as the owner. */
export const signalAutomationStep = (signal: StepSignal) =>
  invoke("automation_signal", { signal: { ...signal, by: "user" } });

interface GitFingerprint {
  head: string | null;
  branch: string | null;
  index: string;
  worktree: string;
}

/** What a `command` attempt recorded: its receipt and the end of its log. */
export interface StepReceipt {
  receipt: {
    attempt: number;
    process_end: string;
    exit_code: number | null;
    duration_ms: number;
    /** Only for steps bound to the Git fingerprint. */
    fingerprint: GitFingerprint | null;
  };
  /** The receipt is an earlier attempt's whose fingerprint still matched. */
  reused: boolean;
  log: string;
  log_truncated: boolean;
}

/** The automation, run and item an attempt belongs to. */
export interface AttemptSource {
  name: string;
  runId: string;
  item: string;
}

/** Null while the attempt has no receipt. */
export const readStepReceipt = (source: AttemptSource, step: string, attempt: number) =>
  invoke<StepReceipt | null>("automation_step_receipt", { ...source, step, attempt });

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

export const isRunActive = (run: AutomationRun) => ACTIVE_RUN_STATUSES.includes(run.status);

export function automationStatus({ entry, state, latestRun }: AutomationSummary): AutomationStatus {
  if (entry.errors.length > 0) return "invalid";
  if (latestRun && isRunActive(latestRun)) return "running";
  if (!state || !state.enabled) return "paused";
  return state.consecutive_failures > 0 ? "degraded" : "active";
}

/** Invalid, failing or auto-paused automations flag the sidebar entry point. */
export function needsAttention({ entry, state }: AutomationSummary): boolean {
  return (
    entry.errors.length > 0 ||
    (state?.consecutive_failures ?? 0) > 0 ||
    state?.pause_reason === "auto"
  );
}

export const AUTOMATION_STATUS_LABELS: Record<AutomationStatus, string> = {
  invalid: "Invalid",
  running: "Running",
  paused: "Paused",
  degraded: "Degraded",
  active: "Active",
};

const ACTOR_LABELS: Record<Actor, string> = { user: "you", cli: "the CLI", auto: "WTM" };

/** Who changed a setting and when. */
export const provenance = ({ by, at }: { by: Actor; at: string }) =>
  `by ${ACTOR_LABELS[by]} on ${new Date(at).toLocaleString()}`;

export function toggleSummary(state: AutomationState | null): string | null {
  const toggle = state?.last_toggle;
  if (!toggle) return state?.pause_reason === "new" && !state.enabled ? "Never enabled" : null;
  const action = toggle.enabled ? "Resumed" : "Paused";
  const when = new Date(toggle.at).toLocaleString();
  const why = toggle.reason ? `: ${toggle.reason}` : "";
  return `${action} by ${ACTOR_LABELS[toggle.by]} on ${when}${why}`;
}

export function triggerSummary(definition: AutomationDefinition): string {
  const { manual, schedule, cli } = definition.trigger;
  if (!schedule && !cli) return manual ? "Manual only" : "No trigger";
  const sources = [
    schedule && `Schedule ${schedule.cron} (${schedule.tz})`,
    manual && "manual",
    cli && "CLI",
  ].filter(Boolean);
  const summary = sources.join(" · ");
  return summary.charAt(0).toUpperCase() + summary.slice(1);
}

export const humanize = (value: string) => value.replace(/_/g, " ");

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

export function formatDuration(ms: number): string {
  const seconds = Math.round(ms / 1000);
  if (seconds < 1) return "<1s";
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}
