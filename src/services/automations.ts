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
    capacity?: boolean;
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
  | { publish: PublishEffect[] }
  | { call: { automation: string; inputs: Record<string, string> } };

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
  /** `complete`: a `noop` attempt completes the item instead of going on. */
  onNoop?: "continue" | "complete";
  /** `none`: the step works in a scratch directory of the run instead of the item's task. */
  task?: "item" | "none";
  kind: StepKind;
}

/** A step of `AUTOMATION.md` as written, with the keys the app does not know. */
export type WrittenStep = Record<string, unknown> & { id?: unknown };

/** The `steps` of an `AUTOMATION.md` and the hash of the file they were read from. */
export interface StepsSource {
  hash: string;
  steps: WrittenStep[];
}

/** A step to save; `source` is its position in the loaded `steps`, null for a new one. */
export interface StepEdit {
  source: number | null;
  value: WrittenStep;
}

/** Nothing is written when the edited file is invalid. */
export type StepsSave =
  | { status: "saved"; entry: AutomationEntry }
  | { status: "invalid"; errors: DefinitionIssue[] };

export const readAutomationSteps = (name: string) =>
  invoke<StepsSource>("automation_steps_source", { name });

/** Rejects when the file changed since `hash` was read. */
export const saveAutomationSteps = (name: string, hash: string, steps: StepEdit[]) =>
  invoke<StepsSave>("automation_steps_save", { name, hash, steps });

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

/** A standing rule the owner recorded; later agent steps follow it. */
export interface Decision {
  id: string;
  text: string;
  by: Actor;
  at: string;
}

/** The run store's `state.json`, serialized in snake_case. */
export interface AutomationState {
  enabled: boolean;
  pause_reason: PauseReason | null;
  next_run_at: string | null;
  consecutive_failures: number;
  last_toggle?: Toggle | null;
  overrides?: Overrides;
  /** Oldest first. */
  decisions?: Decision[];
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
  /** When a wait or approval attempt is due and times out, and the approval's title and detail. */
  wait?: ArmedWait;
  /** The payload that resolved a signal wait, or an approval's answer. */
  signal?: unknown;
}

interface ArmedWait {
  wake_at_ms?: number;
  timeout_at_ms?: number;
  title?: string;
  detail?: string;
  /** The run of another automation a call attempt started and waits on. */
  call?: CalledRun;
}

export interface CalledRun {
  automation: string;
  run_id: string;
}

/** The call attempt of another automation's item that started a run. */
interface CallLink {
  automation: string;
  run_id: string;
  item: string;
  step: string;
  attempt: number;
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
  /** How the asking step ran unattended; missing in runs recorded before it. */
  mode?: "auto" | "strict";
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
  /** When a routine's run last went on after it ended, as a hand back or a retry does. */
  reopened_at_ms?: number | null;
  items: AutomationItem[];
  task_id?: string | null;
  task_error?: { message: string; worktrees: string[] } | null;
  permission_requests?: PermissionRequest[];
  /** Missing unless a call step of another automation started the run. */
  called_by?: CallLink;
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
/** `steered` counts the turns in progress that took the decision at once. */
export const decideForAutomation = (name: string, text: string) =>
  invoke<{ state: AutomationState; decision: Decision; steered: number }>("automation_decide", {
    name,
    text,
    by: "user",
  });

export const revokeAutomationDecision = (name: string, id: string) =>
  invoke<AutomationState>("automation_revoke_decision", { name, id });

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

/** The cause of an item paused for the owner to take over. */
const HUMAN_TAKEOVER = "human_takeover";

export const isTakenOver = (item: AutomationItem) =>
  item.status === "paused" && item.cause === HUMAN_TAKEOVER;

export type ItemAction = "take_over" | "hand_back" | "retry";

const OPEN_ITEM_STATUSES = ["queued", "active", "waiting"];

/**
 * What the owner can do to an item now: take over an agent step with a task it runs or is blocked
 * on, hand back one taken over, retry one that failed or is blocked, or that its routine's failed
 * run left open once it had its task, or without one when its steps need none.
 */
export function itemActions(
  item: AutomationItem,
  steps: PipelineStep[],
  run: AutomationRun | null
): ItemAction[] {
  if (isTakenOver(item)) return ["hand_back"];
  const step = steps.find((candidate) => candidate.id === item.phase);
  const onAgent = !!step && "agent" in step.kind && step.task !== "none";
  const taskless = steps.every((candidate) => candidate.task === "none");
  const leftOpen =
    !!run &&
    (!!run.task_id || taskless) &&
    run.status === "failed" &&
    OPEN_ITEM_STATUSES.includes(item.status);
  const actions: ItemAction[] = [];
  if (onAgent && (item.status === "active" || item.status === "blocked")) actions.push("take_over");
  if (item.status === "failed" || item.status === "blocked" || leftOpen) actions.push("retry");
  return actions;
}

/** What an item action left: the item, and the conversation to continue after a take over. */
export interface ItemControl {
  automation: string;
  run_id: string;
  item: string;
  status: string;
  step: string | null;
  cause: string | null;
  task_id: string | null;
  conversation_id: string | null;
}

export type RetryDefinition = "current" | "original";

const itemTarget = (source: AttemptSource) => ({ ...source, by: "user" });

export const takeOverItem = (source: AttemptSource) =>
  invoke<ItemControl>("automation_take_over", { target: itemTarget(source) });

export const handBackItem = (source: AttemptSource, note: string | null) =>
  invoke<ItemControl>("automation_hand_back", { target: itemTarget(source), note });

export const retryItem = (source: AttemptSource, definition: RetryDefinition) =>
  invoke<ItemControl>("automation_retry", { target: itemTarget(source), definition });

/** What canceling a run left: its status and the items that were still open. */
export interface CanceledRun {
  automation: string;
  run_id: string;
  status: string;
  items: string[];
}

export const cancelRun = (name: string, runId: string) =>
  invoke<CanceledRun>("automation_cancel", { target: { name, runId, by: "user" } });

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
  const { manual, schedule, cli, capacity } = definition.trigger;
  if (!schedule && !cli && !capacity) return manual ? "Manual only" : "No trigger";
  const sources = [
    schedule && `Schedule ${schedule.cron} (${schedule.tz})`,
    manual && "manual",
    cli && "CLI",
    capacity && "freed slots",
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
/** What fired the run; a called run names its caller's automation and run. */
export function runSource(run: AutomationRun): string {
  const source = run.trigger?.source ?? "unknown";
  const caller = run.called_by ? ` from ${calledRunLabel(run.called_by)}` : "";
  return `${source}${caller}${run.trigger?.test ? " · test" : ""}`;
}

/** An automation's run, by its short id. */
export function calledRunLabel({ automation, run_id }: CalledRun): string {
  return `${automation} run ${run_id.split("-")[1] ?? run_id}`;
}

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
