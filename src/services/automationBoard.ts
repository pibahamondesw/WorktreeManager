import { Task } from "../types";
import {
  AutomationDefinition,
  AutomationState,
  Limit,
  Override,
  Pace,
  PipelineStep,
  RunItem,
  StepAttempt,
  humanize,
} from "./automations";
import { ConversationTarget } from "./automationInbox";

export interface BoardCard {
  key: string;
  item: RunItem;
  issue: string | null;
  pr: string | null;
  branch: string | null;
  activeMs: number;
  repairRounds: number;
  /** Why a resting or failed item stopped. */
  reason: string | null;
  /** The attempt the reason comes from, with what it reported. */
  evidence: string | null;
  /** Null when the item has no task yet. */
  target: ConversationTarget | null;
}

export interface BoardColumn {
  id: string;
  label: string;
  cards: BoardCard[];
}

export interface EffectiveLimit {
  limit: Limit;
  label: string;
  value: number | null;
  /** The operator's override, when one replaces the definition's value. */
  override: Override<number> | null;
}

const OPEN_STATUSES = ["queued", "active", "waiting"];
const STOPPED_STATUSES = ["paused", "blocked", "deferred", "failed", "canceled"];
const OPENED_PR_STATUSES = ["executed", "reconciled"];
const STATUS_COLUMNS: { status: string; label: string; always: boolean }[] = [
  { status: "paused", label: "Paused", always: false },
  { status: "blocked", label: "Blocked", always: true },
  { status: "deferred", label: "Deferred", always: true },
  { status: "completed", label: "Integrated", always: true },
  { status: "failed", label: "Failed", always: false },
  { status: "canceled", label: "Canceled", always: false },
];
const LIMIT_LABELS: Record<Limit, string> = {
  max_active: "Max active",
  max_open_prs: "Max open PRs",
  batch_size: "Batch size",
};

const lastOf = <T>(values: T[], matches: (value: T) => boolean) =>
  [...values].reverse().find(matches);

type ColumnPlan = { id: string; label: string; always: boolean };

/**
 * Items by phase: queued, then the step each running or waiting item is on in pipeline order,
 * then the statuses items rest or end in. Columns that only some campaigns reach stay hidden
 * while empty.
 */
export function boardColumns(
  steps: PipelineStep[],
  items: RunItem[],
  tasks: Task[],
  now: number
): BoardColumn[] {
  const plans: ColumnPlan[] = [
    { id: "queued", label: "Queued", always: true },
    ...steps.map((step) => ({ id: `step:${step.id}`, label: step.label, always: true })),
  ];
  const cards = new Map<string, BoardCard[]>();
  for (const item of items) {
    const id = columnOf(item);
    if (!plans.some((plan) => plan.id === id) && id.startsWith("step:"))
      plans.push({ id, label: id.slice(5), always: false });
    cards.set(id, [...(cards.get(id) ?? []), boardCard(steps, item, tasks, now)]);
  }
  plans.push(...STATUS_COLUMNS.map(({ status, label, always }) => ({ id: status, label, always })));
  return plans
    .map(({ id, label, always }) => ({ id, label, always, cards: cards.get(id) ?? [] }))
    .filter(({ always, cards }) => always || cards.length > 0)
    .map(({ id, label, cards }) => ({ id, label, cards }));
}

function columnOf(item: RunItem): string {
  if (item.status === "queued" || (OPEN_STATUSES.includes(item.status) && !item.phase))
    return "queued";
  if (OPEN_STATUSES.includes(item.status)) return `step:${item.phase}`;
  return item.status;
}

export function boardCard(
  steps: PipelineStep[],
  item: RunItem,
  tasks: Task[],
  now: number
): BoardCard {
  const task = tasks.find((candidate) => candidate.id === item.task_id) ?? null;
  const effects = item.steps.flatMap((attempt) => attempt.effects ?? []);
  const reference = (name: string, statuses: string[] | null = null) =>
    lastOf(
      effects,
      (effect) =>
        effect.effect === name &&
        effect.reference !== null &&
        (statuses === null || statuses.includes(effect.status))
    )?.reference ?? null;
  const stopped = STOPPED_STATUSES.includes(item.status);
  return {
    key: `${item.test ? "test:" : ""}${item.id}`,
    item,
    issue: task?.linearIssueIdentifier ?? reference("linear.upsert", OPENED_PR_STATUSES),
    pr: reference("github.pr_create", OPENED_PR_STATUSES),
    branch: task?.branchName ?? null,
    activeMs: activeTime(steps, item, now),
    repairRounds: Object.values(item.counters ?? {}).reduce((sum, rounds) => sum + rounds, 0),
    reason: stopped ? (item.cause ?? null) : null,
    evidence: stopped ? evidenceOf(steps, item.steps) : null,
    target: itemTarget(steps, item),
  };
}

/** Agent and command attempts count, as the `active_time` budget charges them. */
export function activeTime(steps: PipelineStep[], item: RunItem, now: number): number {
  const counted = new Set(
    steps.filter((step) => "agent" in step.kind || "command" in step.kind).map((step) => step.id)
  );
  const running = item.status === "active";
  return item.steps
    .filter((attempt) => counted.has(attempt.step))
    .reduce((total, attempt) => {
      const open = running && attempt.opened_at_ms ? Math.max(0, now - attempt.opened_at_ms) : 0;
      return total + (attempt.elapsed_ms ?? 0) + open;
    }, 0);
}

function evidenceOf(steps: PipelineStep[], attempts: StepAttempt[]): string | null {
  const attempt = lastOf(attempts, (candidate) => !!(candidate.cause || candidate.reply));
  if (!attempt) return null;
  const label = steps.find((step) => step.id === attempt.step)?.label ?? attempt.step;
  const verdict = attempt.verdict ? ` · ${humanize(attempt.verdict)}` : "";
  const detail = [attempt.cause, attempt.reply].filter(Boolean).join("\n");
  return `${label} #${attempt.attempt} (${humanize(attempt.status)}${verdict}): ${detail}`;
}

/** The item's task, at the conversation of its latest agent attempt once the item stops. */
function itemTarget(steps: PipelineStep[], item: RunItem): ConversationTarget | null {
  if (!item.task_id) return null;
  const attempt = lastOf(item.steps, (candidate) => !!candidate.session_id);
  const step = steps.find((candidate) => candidate.id === attempt?.step);
  const agent = step && "agent" in step.kind ? step.kind.agent.agent.id : null;
  const conversationId = OPEN_STATUSES.includes(item.status)
    ? null
    : (attempt?.conversation_id ?? null);
  return { taskId: item.task_id, agent, conversationId };
}

/** The definition's limits, each replaced by the operator's override when there is one. */
export function effectiveLimits(
  definition: AutomationDefinition,
  state: AutomationState | null
): EffectiveLimit[] {
  const defaults: Record<Limit, number | null> = {
    max_active: definition.pools?.active ?? null,
    max_open_prs: definition.pools?.openPrs ?? null,
    batch_size: definition.selector?.maxNewItems ?? null,
  };
  return (Object.keys(LIMIT_LABELS) as Limit[]).map((limit) => {
    const override = state?.overrides?.[limit] ?? null;
    return {
      limit,
      label: LIMIT_LABELS[limit],
      value: override?.value ?? defaults[limit],
      override,
    };
  });
}

export function currentPace(state: AutomationState | null): Pace {
  return state?.overrides?.pace?.value ?? { mode: "continuous" };
}

export function paceLabel(pace: Pace): string {
  if (pace.mode === "continuous") return "Continuous";
  const granted = pace.remaining === 1 ? "1 item granted" : `${pace.remaining} items granted`;
  return `Next · ${granted}`;
}

export const isCampaign = (definition: AutomationDefinition | null) => !!definition?.selector;
