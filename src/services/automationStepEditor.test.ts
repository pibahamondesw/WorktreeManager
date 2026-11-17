import { describe, expect, it } from "vitest";
import {
  DraftStep,
  addStep,
  changeKind,
  draftKind,
  draftSteps,
  kindField,
  moveStep,
  previewSteps,
  removeStep,
  renameStep,
  setField,
  stepEdits,
  transitionDraft,
  updateStep,
  writtenTransition,
} from "./automationStepEditor";
import { WrittenStep } from "./automations";

const WRITTEN: WrittenStep[] = [
  { id: "implement", agent: { skill: "build" }, "x-owner": "me" },
  {
    id: "verify",
    command: ["scripts/verify"],
    repo: "app",
    on_fail: { goto: "implement", max: 3 },
  },
  {
    id: "review",
    agent: { on_reject: { goto: "implement", max: "repair_rounds" } },
    next: "verify",
  },
];

const draft = () => draftSteps({ hash: "h", steps: WRITTEN });
const values = (steps: DraftStep[]) => steps.map((step) => step.value);

describe("draft steps", () => {
  it("keep each written step with its source and unknown keys", () => {
    const steps = draft();
    expect(steps.map((step) => step.source)).toEqual([0, 1, 2]);
    expect(new Set(steps.map((step) => step.key)).size).toBe(3);
    expect(stepEdits(steps)[0]).toEqual({ source: 0, value: WRITTEN[0] });
  });

  it("write out the implicit agent step of a file without steps", () => {
    expect(stepEdits(draftSteps({ hash: "h", steps: [] }))).toEqual([
      { source: null, value: { id: "main", agent: {} } },
    ]);
  });

  it("add a step after the selected one with a free id and default config", () => {
    const { steps, index } = addStep(draft(), "approval", 0);
    expect(index).toBe(1);
    expect(steps[1]).toMatchObject({
      source: null,
      value: { id: "approval_4", approval: { title: "Approve approval_4?" } },
    });
    const taken = addStep(
      draftSteps({ hash: "h", steps: [{ id: "wait_2", wait: {} }] }),
      "wait",
      null
    );
    expect(taken.steps[1].value).toEqual({ id: "wait_3", wait: { signal: "wait_3" } });
    expect(addStep(draft(), "agent", null).steps[3].value).toEqual({ id: "agent_4", agent: {} });
    expect(addStep(draft(), "command", null).steps[3].value.command).toEqual([]);
  });

  it("move within bounds and remove steps", () => {
    const steps = draft();
    expect(values(moveStep(steps, 0, 1)).map((step) => step.id)).toEqual([
      "verify",
      "implement",
      "review",
    ]);
    expect(moveStep(steps, 0, -1)).toBe(steps);
    expect(moveStep(steps, 2, 1)).toBe(steps);
    expect(values(removeStep(steps, 1)).map((step) => step.id)).toEqual(["implement", "review"]);
    expect(updateStep(steps, 1, { id: "x", agent: {} })[1]).toMatchObject({
      source: 1,
      value: { id: "x" },
    });
  });

  it("rename a step and every jump to it", () => {
    const renamed = values(renameStep(draft(), 0, "build"));
    expect(renamed[0].id).toBe("build");
    expect(renamed[1].on_fail).toEqual({ goto: "build", max: 3 });
    expect(renamed[2].agent).toEqual({ on_reject: { goto: "build", max: "repair_rounds" } });
    expect(values(renameStep(draft(), 1, "check"))[2].next).toBe("check");
    const steps = draft();
    expect(renameStep(steps, 0, "implement")).toBe(steps);
  });
});

describe("step fields", () => {
  it("set and clear step and kind keys", () => {
    const step = WRITTEN[0];
    expect(setField(step, "label", "Build")).toEqual({ ...step, label: "Build" });
    expect(setField({ ...step, label: "Build" }, "label", "")).toEqual(step);
    expect(setField(step, "model", "opus", true).agent).toEqual({ skill: "build", model: "opus" });
    expect(setField(step, "skill", undefined, true).agent).toEqual({});
    expect(setField({ id: "a", agent: null }, "model", "opus", true).agent).toEqual({
      model: "opus",
    });
    expect(setField({ id: "a" }, "model", "opus", true)).toEqual({ id: "a", model: "opus" });
    expect(kindField(step, "skill")).toBe("build");
    expect(kindField(WRITTEN[1], "skill")).toBeUndefined();
    expect(kindField({ id: "a" }, "skill")).toBeUndefined();
  });

  it("name the kind only when the step has exactly one", () => {
    expect(draftKind(WRITTEN[1])).toBe("command");
    expect(draftKind({ id: "a", agent: {}, wait: {} })).toBeNull();
    expect(draftKind({ id: "a" })).toBeNull();
  });

  it("change kind dropping the old kind and its keys", () => {
    expect(changeKind(WRITTEN[1], "agent")).toEqual({
      id: "verify",
      on_fail: { goto: "implement", max: 3 },
      agent: {},
    });
    expect(changeKind(WRITTEN[0], "wait")).toEqual({
      id: "implement",
      "x-owner": "me",
      wait: { signal: "implement" },
    });
    expect(changeKind(WRITTEN[0], "agent")).toBe(WRITTEN[0]);
    expect(changeKind({ id: "a" }, "approval")).toEqual({
      id: "a",
      approval: { title: "Approve a?" },
    });
  });

  it("read and write transitions", () => {
    expect(transitionDraft(undefined)).toEqual({ mode: "default", step: "", max: "" });
    expect(transitionDraft("block")).toEqual({ mode: "block", step: "", max: "" });
    expect(transitionDraft({ goto: "a", max: 2 })).toEqual({ mode: "goto", step: "a", max: "2" });
    expect(transitionDraft({ goto: "a", max: "repair_rounds" }).max).toBe("repair_rounds");
    expect(transitionDraft({ goto: "a" }).max).toBe("");
    expect(writtenTransition({ mode: "default", step: "", max: "" })).toBeUndefined();
    expect(writtenTransition({ mode: "defer", step: "a", max: "2" })).toBe("defer");
    expect(writtenTransition({ mode: "goto", step: "a", max: " " })).toEqual({ goto: "a" });
    expect(writtenTransition({ mode: "goto", step: "a", max: "4" })).toEqual({ goto: "a", max: 4 });
    expect(writtenTransition({ mode: "goto", step: "a", max: "repair_rounds" })).toEqual({
      goto: "a",
      max: "repair_rounds",
    });
  });
});

describe("preview", () => {
  it("lays the draft out as pipeline steps", () => {
    const preview = previewSteps(
      draftSteps({
        hash: "h",
        steps: [
          ...WRITTEN,
          { id: "pause", label: "Pause", wait: { time: "1h" } },
          { id: "ready", wait: { signal: "go" }, on_fail: "block" },
          { id: "ship", approval: { title: "Ship?", on_reject: "defer" } },
          { id: "publish", publish: ["git.push", { "github.pr_create": { draft: true } }, 3] },
          { id: "single", publish: "wtm.task_log" },
          { id: "broken", command: "not a list" },
          { id: "delegate", call: { automation: "fix", inputs: { issue: "{item.id}", n: 2 } } },
          { id: "bare", call: { automation: "fix" } },
        ],
      })
    );
    expect(preview[0]).toEqual({
      id: "implement",
      label: "implement",
      next: null,
      onFail: "fail",
      kind: {
        agent: {
          agent: { id: "default agent", model: "", effort: null },
          skill: "build",
          prompt: null,
          onReject: null,
        },
      },
    });
    expect(preview[1].onFail).toEqual({ goto: { step: "implement", max: { count: 3 } } });
    expect(preview[1].kind).toEqual({ command: { command: ["scripts/verify"], repo: "" } });
    expect(preview[2]).toMatchObject({
      next: "verify",
      kind: {
        agent: { onReject: { goto: { step: "implement", max: { budget: "repair_rounds" } } } },
      },
    });
    expect(preview[3]).toMatchObject({ label: "Pause", kind: { wait: { until: "1h" } } });
    expect(preview[4]).toMatchObject({ onFail: "block", kind: { wait: { signal: "go" } } });
    expect(preview[5].kind).toEqual({
      approval: { title: "Ship?", detail: null, onReject: "defer" },
    });
    expect(preview[6].kind).toEqual({
      publish: [{ effect: "git.push" }, { effect: "github.pr_create" }, { effect: "" }],
    });
    expect(preview[7].kind).toEqual({ publish: [{ effect: "wtm.task_log" }] });
    expect(preview[8].kind).toEqual({ command: { command: [], repo: "" } });
    expect(preview[9].kind).toEqual({
      call: { automation: "fix", inputs: { issue: "{item.id}", n: "2" } },
    });
    expect(preview[10].kind).toEqual({ call: { automation: "fix", inputs: {} } });
    expect(previewSteps(draftSteps({ hash: "h", steps: [{ id: "x", wait: {} }] }))[0].kind).toEqual(
      {
        wait: { signal: "" },
      }
    );
    expect(
      previewSteps(
        draftSteps({ hash: "h", steps: [{ id: "x", agent: { on_reject: { goto: "x" } } }] })
      )[0].kind
    ).toMatchObject({ agent: { onReject: { goto: { step: "x", max: null } } } });
  });
});
