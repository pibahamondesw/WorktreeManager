import {
  AutomationItem,
  PipelineStep,
  StepAttempt,
  StepStatus,
  Transition,
  WaitCondition,
  formatDuration,
} from "./automations";

export type EdgeKind = "next" | "on_fail" | "on_reject";

/** A jump between steps that are not consecutive, drawn in its own lane beside the steps. */
export interface PipelineEdge {
  from: number;
  to: number;
  kind: EdgeKind;
  limit: string | null;
  lane: number;
}

export interface PipelineLayout {
  edges: PipelineEdge[];
  lanes: number;
}

export type StepState = StepStatus | "not_started";

export interface StepProgress {
  state: StepState;
  attempts: number;
  latest: StepAttempt | null;
  current: boolean;
}

const TERMINAL_ITEM_STATUSES = ["completed", "failed", "canceled"];

export const stepKindName = (step: PipelineStep) => Object.keys(step.kind)[0];

export function onReject(step: PipelineStep): Transition | null {
  if ("agent" in step.kind) return step.kind.agent.onReject;
  if ("approval" in step.kind) return step.kind.approval.onReject;
  return null;
}

export function pipelineLayout(steps: PipelineStep[]): PipelineLayout {
  const indexOf = new Map(steps.map((step, index) => [step.id, index]));
  const jumps = steps.flatMap((step, from) => {
    const targets: [EdgeKind, string | null, string | null][] = [
      ["next", step.next, null],
      ...transitionTarget("on_fail", step.onFail),
      ...transitionTarget("on_reject", onReject(step)),
    ];
    return targets.flatMap(([kind, target, limit]) => {
      const to = target === null ? undefined : indexOf.get(target);
      if (to === undefined || (kind === "next" && to === from + 1)) return [];
      return [{ from, to, kind, limit }];
    });
  });
  return assignLanes(jumps);
}

function transitionTarget(
  kind: EdgeKind,
  transition: Transition | null
): [EdgeKind, string, string | null][] {
  if (transition === null || typeof transition === "string") return [];
  return [[kind, transition.goto.step, gotoLimit(transition)]];
}

function gotoLimit(transition: Extract<Transition, { goto: unknown }>): string | null {
  const { max } = transition.goto;
  if (max === null) return null;
  return "count" in max ? `max ${max.count}` : `budget ${max.budget}`;
}

/** Shorter jumps take the inner lanes; overlapping jumps never share a lane. */
function assignLanes(jumps: Omit<PipelineEdge, "lane">[]): PipelineLayout {
  const span = ({ from, to }: { from: number; to: number }) => [
    Math.min(from, to),
    Math.max(from, to),
  ];
  const byLength = [...jumps].sort((a, b) => Math.abs(a.to - a.from) - Math.abs(b.to - b.from));
  const lanes: number[][][] = [];
  const edges = byLength.map((jump) => {
    const [low, high] = span(jump);
    let lane = lanes.findIndex((taken) => taken.every(([l, h]) => high < l || h < low));
    if (lane === -1) lane = lanes.push([]) - 1;
    lanes[lane].push([low, high]);
    return { ...jump, lane };
  });
  return { edges, lanes: lanes.length };
}

export function stepProgress(
  steps: PipelineStep[],
  item: AutomationItem | null
): Map<string, StepProgress> {
  const open = item !== null && !TERMINAL_ITEM_STATUSES.includes(item.status);
  return new Map(
    steps.map((step) => {
      const attempts = item?.steps.filter((attempt) => attempt.step === step.id) ?? [];
      const latest = attempts.reduce<StepAttempt | null>(
        (newest, attempt) => (newest && newest.attempt > attempt.attempt ? newest : attempt),
        null
      );
      return [
        step.id,
        {
          state: latest?.status ?? "not_started",
          attempts: attempts.length,
          latest,
          current: open && item.phase === step.id,
        },
      ];
    })
  );
}

export function transitionLabel(transition: Transition): string {
  if (typeof transition === "string") return transition;
  const limit = gotoLimit(transition);
  return limit ? `goto ${transition.goto.step} (${limit})` : `goto ${transition.goto.step}`;
}

export function waitConditionLabel(condition: WaitCondition): string {
  if ("duration_secs" in condition) return `For ${formatDuration(condition.duration_secs * 1000)}`;
  if ("until" in condition) return `Until ${condition.until}`;
  if ("event" in condition) return `Event ${condition.event.watcher}`;
  return `Signal ${condition.signal}`;
}

/** When a wait or approval attempt is due and times out, what it was signaled and what its
 * watcher decided, such as `staged` in a Test run. */
export function waitFacts(attempt: StepAttempt | null): [label: string, value: string][] {
  const wait = attempt?.wait;
  if (!attempt || !wait) return [];
  const at = (ms: number) => new Date(ms).toLocaleString();
  const facts: [string, string][] = [];
  if (wait.wake_at_ms != null) facts.push(["Due", at(wait.wake_at_ms)]);
  if (wait.timeout_at_ms != null) facts.push(["Times out", at(wait.timeout_at_ms)]);
  if (attempt.signal != null) facts.push(["Signal", JSON.stringify(attempt.signal)]);
  const watched = watcherVerdict(attempt.result);
  if (watched) facts.push(["Watcher", watched]);
  return facts;
}

function watcherVerdict(result: unknown): string | null {
  if (typeof result !== "object" || result === null) return null;
  const { verdict, detail } = result as { verdict?: unknown; detail?: unknown };
  if (typeof verdict !== "string") return null;
  return typeof detail === "string" ? `${verdict} · ${detail}` : verdict;
}

/** An approval's run title replaces its template once the step has run. */
export function stepSummary(step: PipelineStep, attempt: StepAttempt | null = null): string {
  const { kind } = step;
  if ("agent" in kind) return [kind.agent.agent.id, kind.agent.skill].filter(Boolean).join(" · ");
  if ("command" in kind) return kind.command.command.join(" ");
  if ("wait" in kind) return waitConditionLabel(kind.wait);
  if ("approval" in kind) return attempt?.wait?.title ?? kind.approval.title;
  return kind.publish.map((effect) => effect.effect).join(" · ");
}
