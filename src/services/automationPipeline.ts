import {
  AutomationItem,
  PipelineStep,
  StepAttempt,
  StepStatus,
  Transition,
  WaitCondition,
  calledRunLabel,
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

type TransitionEnd = "done" | "fail" | "block" | "defer";

/** Where a step goes on success (`next`), failure or rejection: another step or the item's end. */
export interface PipelineTransition {
  from: string;
  kind: EdgeKind;
  to: { step: string } | { end: TransitionEnd };
  limit: string | null;
}

/** Every transition of the definition, independent of how it is drawn. A rejection without its
 * own `on_reject` follows `on_fail`, and `continue` follows `next`. */
export function pipelineTransitions(steps: PipelineStep[]): PipelineTransition[] {
  return steps.flatMap((step, index) => {
    const next: PipelineTransition["to"] = step.next
      ? { step: step.next }
      : index + 1 < steps.length
        ? { step: steps[index + 1].id }
        : { end: "done" };
    const follow = (kind: EdgeKind, transition: Transition): PipelineTransition => {
      if (typeof transition !== "string") {
        return {
          from: step.id,
          kind,
          to: { step: transition.goto.step },
          limit: gotoLimit(transition),
        };
      }
      return {
        from: step.id,
        kind,
        to: transition === "continue" ? next : { end: transition },
        limit: null,
      };
    };
    const reject = onReject(step);
    return [
      { from: step.id, kind: "next" as const, to: next, limit: null },
      follow("on_fail", step.onFail),
      ...(reject === null ? [] : [follow("on_reject", reject)]),
    ];
  });
}

export const transitionKey = ({ from, kind }: Pick<PipelineTransition, "from" | "kind">) =>
  `${from}:${kind}`;

/** How many times the item took each transition between steps, keyed by `transitionKey`, read
 * from its attempts in the order they opened. An attempt of a step the definition no longer
 * has breaks the path there. */
export function traversedTransitions(
  steps: PipelineStep[],
  item: AutomationItem | null
): Map<string, number> {
  const transitions = pipelineTransitions(steps);
  const counts = new Map<string, number>();
  const attempts = item?.steps ?? [];
  attempts.slice(1).forEach((reached, index) => {
    const left = attempts[index];
    const outgoing = transitions.filter(
      ({ from, to }) => from === left.step && "step" in to && to.step === reached.step
    );
    const taken =
      attemptExits(left)
        .map((kind) => outgoing.find((transition) => transition.kind === kind))
        .find(Boolean) ?? outgoing[0];
    if (taken) counts.set(transitionKey(taken), (counts.get(transitionKey(taken)) ?? 0) + 1);
  });
  return counts;
}

function attemptExits(attempt: StepAttempt): EdgeKind[] {
  if (attempt.status === "failed") return ["on_fail"];
  if (attempt.status === "rejected") return ["on_reject", "on_fail"];
  return ["next"];
}

/** Steps an item ran that the definition no longer has, in the order the item reached them. */
export function unknownAttemptSteps(steps: PipelineStep[], item: AutomationItem | null): string[] {
  const known = new Set(steps.map((step) => step.id));
  return [...new Set((item?.steps ?? []).map((attempt) => attempt.step))].filter(
    (id) => !known.has(id)
  );
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
  if ("call" in kind) {
    const called = attempt?.wait?.call;
    return called ? calledRunLabel(called) : kind.call.automation;
  }
  return kind.publish.map((effect) => effect.effect).join(" · ");
}
