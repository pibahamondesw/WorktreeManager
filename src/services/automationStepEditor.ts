import {
  PipelineStep,
  StepEdit,
  StepKind,
  StepsSource,
  Transition,
  WrittenStep,
} from "./automations";

/** A step of the editor's draft: its `AUTOMATION.md` map, where it came from and a stable key. */
export interface DraftStep {
  key: string;
  /** Position in the loaded `steps`; null for a new step. */
  source: number | null;
  value: WrittenStep;
}

export type EditableKind = "agent" | "command" | "wait" | "approval";
/** Kinds the editor keeps as written. */
type WrittenKind = "publish" | "call";
type KindName = EditableKind | WrittenKind;

export const EDITABLE_KINDS: EditableKind[] = ["agent", "command", "wait", "approval"];
const KIND_NAMES: KindName[] = [...EDITABLE_KINDS, "publish", "call"];

/** Why a step of a kind the editor keeps as written has no fields here. */
export const WRITTEN_KIND_NOTES: Record<WrittenKind, string> = {
  publish: "Publish effects are edited in AUTOMATION.md and kept as written.",
  call: "The called automation and its inputs are edited in AUTOMATION.md and kept as written.",
};

export const isWrittenKind = (kind: KindName | null): kind is WrittenKind =>
  kind === "publish" || kind === "call";
/** Step keys that only mean something for one kind, dropped when the kind changes. */
const KIND_SCOPED_KEYS: Partial<Record<KindName, string[]>> = {
  command: ["repo", "env", "bind", "after"],
};
const IMPLICIT_STEP: WrittenStep = { id: "main", agent: {} };

export type TransitionMode = "default" | "fail" | "block" | "defer" | "continue" | "goto";

/** A transition as the editor shows it; `max` is the text of a count or a budget name. */
export interface TransitionDraft {
  mode: TransitionMode;
  step: string;
  max: string;
}

let nextKey = 0;
const draftKey = () => `draft-${++nextKey}`;

const isMap = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const text = (value: unknown) => (typeof value === "string" ? value : "");

/** A file without `steps` runs one implicit agent step, which the draft writes out. */
export function draftSteps(source: StepsSource): DraftStep[] {
  if (source.steps.length === 0) {
    return [{ key: draftKey(), source: null, value: structuredClone(IMPLICIT_STEP) }];
  }
  return source.steps.map((value, index) => ({ key: draftKey(), source: index, value }));
}

export const stepEdits = (steps: DraftStep[]): StepEdit[] =>
  steps.map(({ source, value }) => ({ source, value }));

export function draftKind(step: WrittenStep): KindName | null {
  const present = KIND_NAMES.filter((kind) => kind in step);
  return present.length === 1 ? present[0] : null;
}

function uniqueId(steps: DraftStep[], base: string) {
  const taken = new Set(steps.map((step) => step.value.id));
  let index = steps.length + 1;
  while (taken.has(`${base}_${index}`)) index += 1;
  return `${base}_${index}`;
}

function defaultConfig(kind: EditableKind, id: string): unknown {
  if (kind === "agent") return {};
  if (kind === "command") return [];
  if (kind === "wait") return { signal: id };
  return { title: `Approve ${id}?` };
}

/** Inserts a new step of `kind` right after `after`, or at the end. */
export function addStep(steps: DraftStep[], kind: EditableKind, after: number | null) {
  const id = uniqueId(steps, kind);
  const step: DraftStep = {
    key: draftKey(),
    source: null,
    value: { id, [kind]: defaultConfig(kind, id) },
  };
  const at = after === null ? steps.length : after + 1;
  return { steps: [...steps.slice(0, at), step, ...steps.slice(at)], index: at };
}

export const removeStep = (steps: DraftStep[], index: number) =>
  steps.filter((_, position) => position !== index);

export function moveStep(steps: DraftStep[], index: number, offset: number): DraftStep[] {
  const target = index + offset;
  if (target < 0 || target >= steps.length) return steps;
  const moved = [...steps];
  [moved[index], moved[target]] = [moved[target], moved[index]];
  return moved;
}

/** Replaces the step's map; the step keeps its key and source. */
export const updateStep = (steps: DraftStep[], index: number, value: WrittenStep) =>
  steps.map((step, position) => (position === index ? { ...step, value } : step));

/** Sets `key` of the step, or of its kind's map when `inKind`; an empty value removes the key. */
export function setField(step: WrittenStep, key: string, value: unknown, inKind = false) {
  const kind = draftKind(step);
  if (inKind && kind) {
    const config = isMap(step[kind]) ? { ...(step[kind] as Record<string, unknown>) } : {};
    return { ...step, [kind]: withValue(config, key, value) };
  }
  return withValue({ ...step }, key, value) as WrittenStep;
}

function withValue(map: Record<string, unknown>, key: string, value: unknown) {
  if (value === undefined || value === "") delete map[key];
  else map[key] = value;
  return map;
}

export function kindField(step: WrittenStep, key: string): unknown {
  const kind = draftKind(step);
  const config = kind ? step[kind] : null;
  return isMap(config) ? config[key] : undefined;
}

/** Another kind with its default config; keys only the old kind reads are dropped. */
export function changeKind(step: WrittenStep, kind: EditableKind): WrittenStep {
  const current = draftKind(step);
  if (current === kind) return step;
  const changed = { ...step };
  for (const name of KIND_NAMES) delete changed[name];
  for (const key of (current && KIND_SCOPED_KEYS[current]) ?? []) delete changed[key];
  return { ...changed, [kind]: defaultConfig(kind, text(step.id)) };
}

/** Renames a step and every `next` and `goto` that pointed at its old id. */
export function renameStep(steps: DraftStep[], index: number, id: string): DraftStep[] {
  const old = steps[index].value.id;
  if (old === id) return steps;
  const retarget = (value: unknown) =>
    isMap(value) && value.goto === old ? { ...value, goto: id } : value;
  return steps.map((step, position) => {
    const value: WrittenStep = { ...step.value };
    if (position === index) value.id = id;
    if (value.next === old) value.next = id;
    if ("on_fail" in value) value.on_fail = retarget(value.on_fail);
    const kind = draftKind(value);
    const config = kind ? value[kind] : null;
    if (kind && isMap(config) && "on_reject" in config) {
      value[kind] = { ...config, on_reject: retarget(config.on_reject) };
    }
    return { ...step, value };
  });
}

export function transitionDraft(value: unknown): TransitionDraft {
  if (isMap(value)) {
    const max = value.max;
    return {
      mode: "goto",
      step: text(value.goto),
      max: typeof max === "number" || typeof max === "string" ? String(max) : "",
    };
  }
  const mode = (["fail", "block", "defer", "continue"] as const).find((name) => name === value);
  return { mode: mode ?? "default", step: "", max: "" };
}

/** The YAML value of a transition draft; undefined leaves the key out. */
export function writtenTransition({ mode, step, max }: TransitionDraft): unknown {
  if (mode === "default") return undefined;
  if (mode !== "goto") return mode;
  const limit = max.trim();
  if (!limit) return { goto: step };
  return { goto: step, max: /^\d+$/.test(limit) ? Number(limit) : limit };
}

function previewTransition(value: unknown): Transition | null {
  const { mode, step, max } = transitionDraft(value);
  if (mode === "default") return null;
  if (mode !== "goto") return mode;
  const limit = max === "" ? null : /^\d+$/.test(max) ? { count: Number(max) } : { budget: max };
  return { goto: { step, max: limit } };
}

const effectName = (effect: unknown) =>
  typeof effect === "string" ? effect : isMap(effect) ? (Object.keys(effect)[0] ?? "") : "";

function previewKind(step: WrittenStep): StepKind {
  const kind = draftKind(step);
  const config = kind ? step[kind] : undefined;
  const field = (key: string) => (isMap(config) ? config[key] : undefined);
  if (kind === "command") {
    return { command: { command: Array.isArray(config) ? config.map(String) : [], repo: "" } };
  }
  if (kind === "wait") {
    const signal = text(field("signal"));
    return { wait: field("time") ? { until: String(field("time")) } : { signal } };
  }
  if (kind === "approval") {
    return {
      approval: {
        title: text(field("title")),
        detail: null,
        onReject: previewTransition(field("on_reject")),
      },
    };
  }
  if (kind === "call") {
    const inputs = isMap(field("inputs")) ? (field("inputs") as Record<string, unknown>) : {};
    const written = Object.entries(inputs).map(([name, value]): [string, string] => [
      name,
      String(value),
    ]);
    return { call: { automation: text(field("automation")), inputs: Object.fromEntries(written) } };
  }
  if (kind === "publish") {
    const effects = Array.isArray(config) ? config : [config];
    return { publish: effects.map((effect) => ({ effect: effectName(effect) })) };
  }
  return {
    agent: {
      agent: {
        id: text(field("id")) || "default agent",
        model: text(field("model")),
        effort: null,
      },
      skill: text(field("skill")) || null,
      prompt: text(field("prompt")) || null,
      onReject: previewTransition(field("on_reject")),
    },
  };
}

/** The draft as pipeline steps, to lay out and summarize it like a loaded definition. */
export const previewSteps = (steps: DraftStep[]): PipelineStep[] =>
  steps.map(({ value }) => ({
    id: text(value.id),
    label: text(value.label) || text(value.id),
    next: text(value.next) || null,
    onFail: previewTransition(value.on_fail) ?? "fail",
    kind: previewKind(value),
  }));
