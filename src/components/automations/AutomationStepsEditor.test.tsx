// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { invoke } from "@tauri-apps/api/core";
import { AutomationStepsEditor } from "./AutomationStepsEditor";
import { AutomationPipeline } from "./AutomationPipeline";
import { StepEdit, WrittenStep } from "../../services/automations";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const STEPS: WrittenStep[] = [
  { id: "implement", agent: { skill: "build" }, "x-owner": "me" },
  { id: "verify", command: ["scripts/verify"], on_fail: { goto: "implement", max: 3 } },
  { id: "approve", approval: { title: "Ship?" } },
  { id: "checks", wait: { event: { watcher: "pr_checks" } } },
  {
    id: "ship",
    publish: [
      "git.push",
      { "wtm.chat_open": { agent: "claude", message: "Review", with_template: true } },
    ],
  },
];

type Saver = (args: { name: string; hash: string; steps: StepEdit[] }) => unknown;

function mockBackend(save: Saver = () => ({ status: "saved", entry: {} }), steps = STEPS) {
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "agents_catalog") {
      return [{ id: "claude", label: "Claude", installed: true, capabilities: {} }];
    }
    if (command === "automation_steps_source") return { hash: "h1", steps };
    if (command === "automation_steps_save") return save(args as Parameters<Saver>[0]);
    if (command === "automation_runs_list") return { runs: [], next_before: null };
    return null;
  });
}

const savedSteps = () =>
  (
    vi.mocked(invoke).mock.calls.find(([command]) => command === "automation_steps_save")?.[1] as {
      steps: StepEdit[];
    }
  ).steps;

async function renderEditor(props: Partial<Parameters<typeof AutomationStepsEditor>[0]> = {}) {
  const handlers = { onClose: vi.fn(), onSaved: vi.fn() };
  render(<AutomationStepsEditor name="demo" {...handlers} {...props} />);
  await screen.findByRole("list", { name: "Draft steps" });
  return handlers;
}

const node = (label: string) =>
  within(screen.getByRole("list", { name: "Draft steps" })).getByRole("button", {
    name: new RegExp(`^${label}`),
  });

const settings = () => within(screen.getByRole("region", { name: "Step settings" }));

beforeEach(() => vi.mocked(invoke).mockReset());
afterEach(cleanup);

it("configures an agent step and saves every step with its source", async () => {
  mockBackend();
  const { onSaved } = await renderEditor();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  await screen.findByRole("option", { name: "Claude" });

  fireEvent.change(settings().getByLabelText("Agent"), { target: { value: "claude" } });
  fireEvent.change(settings().getByLabelText("Model"), { target: { value: "opus" } });
  fireEvent.change(settings().getByLabelText("Effort"), { target: { value: "high" } });
  fireEvent.change(settings().getByLabelText("Skill"), { target: { value: "" } });
  fireEvent.change(settings().getByLabelText("Prompt"), { target: { value: "Keep it small" } });
  fireEvent.change(settings().getByLabelText("Label"), { target: { value: "Build" } });
  fireEvent.change(settings().getByLabelText("On reject"), { target: { value: "block" } });
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save" })));

  expect(savedSteps()[0]).toEqual({
    source: 0,
    value: {
      id: "implement",
      label: "Build",
      "x-owner": "me",
      agent: {
        id: "claude",
        model: "opus",
        effort: "high",
        prompt: "Keep it small",
        on_reject: "block",
      },
    },
  });
  expect(savedSteps().map((step) => step.source)).toEqual([0, 1, 2, 3, 4]);
  expect(savedSteps()[4]).toEqual({ source: 4, value: STEPS[4] });
  expect(vi.mocked(invoke)).toHaveBeenCalledWith(
    "automation_steps_save",
    expect.objectContaining({ name: "demo", hash: "h1" })
  );
  expect(onSaved).toHaveBeenCalled();
});

it("adds, connects, reorders and removes steps from the graph", async () => {
  mockBackend();
  await renderEditor();

  fireEvent.change(screen.getByLabelText("New step type"), { target: { value: "wait" } });
  fireEvent.click(screen.getByRole("button", { name: "Add step" }));
  expect(node("wait_6")).toHaveAttribute("aria-pressed", "true");
  fireEvent.change(settings().getByLabelText("Wait for"), { target: { value: "time" } });
  fireEvent.change(settings().getByLabelText("Duration or instant"), { target: { value: "45m" } });
  fireEvent.change(settings().getByLabelText("Next"), { target: { value: "approve" } });
  fireEvent.change(settings().getByLabelText("On fail"), { target: { value: "goto" } });
  fireEvent.change(settings().getByLabelText("On fail step"), { target: { value: "verify" } });
  fireEvent.change(settings().getByLabelText("On fail max"), { target: { value: "2" } });
  fireEvent.click(settings().getByRole("button", { name: "Move up" }));
  fireEvent.keyDown(node("wait_6"), { key: "ArrowUp" });
  fireEvent.keyDown(node("wait_6"), { key: "ArrowDown" });
  expect(node("implement")).toHaveAttribute("aria-pressed", "true");
  fireEvent.keyDown(node("implement"), { key: "ArrowDown", altKey: true });
  fireEvent.keyDown(node("implement"), { key: "ArrowUp", altKey: true });
  fireEvent.keyDown(node("implement"), { key: "ArrowUp" });
  fireEvent.keyDown(node("implement"), { key: "Enter" });
  fireEvent.click(node("ship"));
  expect(settings().getByText(/Publish effects are edited in AUTOMATION.md/)).toBeInTheDocument();
  expect(settings().getByRole("button", { name: "Move down" })).toBeDisabled();
  fireEvent.click(settings().getByRole("button", { name: "Remove" }));
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save" })));

  expect(savedSteps().map(({ source, value }) => [source, value.id])).toEqual([
    [null, "wait_6"],
    [0, "implement"],
    [1, "verify"],
    [2, "approve"],
    [3, "checks"],
  ]);
  expect(savedSteps()[0].value).toEqual({
    id: "wait_6",
    wait: { time: "45m" },
    next: "approve",
    on_fail: { goto: "verify", max: 2 },
  });
  expect(screen.getByRole("list", { name: "Jumps" })).toHaveTextContent("wait_6 → approve");
});

it("renames a step and its jumps, and edits command, approval and wait steps", async () => {
  mockBackend();
  await renderEditor();
  const id = settings().getByLabelText("Id");
  fireEvent.change(id, { target: { value: "build" } });
  fireEvent.keyDown(id, { key: "Enter" });
  fireEvent.blur(id);

  fireEvent.click(node("verify"));
  expect(settings().getByLabelText("On fail step")).toHaveValue("build");
  fireEvent.change(settings().getByLabelText("Command (one argument per line)"), {
    target: { value: "scripts/verify\n--fast\n" },
  });
  fireEvent.change(settings().getByLabelText("Type"), { target: { value: "approval" } });
  fireEvent.change(settings().getByLabelText("Title"), { target: { value: "Verified?" } });
  fireEvent.click(node("approve"));
  fireEvent.change(settings().getByLabelText("On reject"), { target: { value: "goto" } });
  fireEvent.click(node("checks"));
  expect(settings().getByText(/The watcher and its params/)).toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save" })));

  const [build, verify, approve] = savedSteps().map((step) => step.value);
  expect(build.id).toBe("build");
  expect(verify).toEqual({
    id: "verify",
    approval: { title: "Verified?" },
    on_fail: { goto: "build", max: 3 },
  });
  expect(approve.approval).toEqual({ title: "Ship?", on_reject: { goto: "build" } });
});

it("keeps the command lines as typed", async () => {
  mockBackend(undefined, [{ id: "verify", command: "scripts/verify" }]);
  await renderEditor();
  const command = settings().getByLabelText("Command (one argument per line)");
  expect(command).toHaveValue("");
  fireEvent.change(command, { target: { value: "a\n" } });
  expect(command).toHaveValue("a\n");
  expect(settings().queryByRole("button", { name: "Remove" })).toBeNull();
});

it("shows validation errors without closing", async () => {
  mockBackend(() => ({
    status: "invalid",
    errors: [
      { path: "/a", field: "steps[0].next", kind: "invalid", message: "unknown step nowhere" },
      { path: "/a", field: "", kind: "invalid", message: "whole file" },
    ],
  }));
  const { onSaved } = await renderEditor();
  fireEvent.change(settings().getByLabelText("Next"), { target: { value: "verify" } });
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save" })));

  const errors = screen.getByRole("list", { name: "Validation errors" });
  expect(errors).toHaveTextContent("steps[0].next: unknown step nowhere");
  expect(errors).toHaveTextContent("file: whole file");
  expect(onSaved).not.toHaveBeenCalled();
  fireEvent.change(settings().getByLabelText("Next"), { target: { value: "" } });
  expect(screen.queryByRole("list", { name: "Validation errors" })).toBeNull();
});

it("reports a file changed on disk and reloads it on request", async () => {
  mockBackend(() => Promise.reject("AUTOMATION.md changed on disk since the editor loaded it"));
  await renderEditor();
  fireEvent.change(settings().getByLabelText("Label"), { target: { value: "Build" } });
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save" })));
  expect(screen.getByRole("alert")).toHaveTextContent("changed on disk");

  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Reload from disk" })));
  expect(screen.queryByRole("alert")).toBeNull();
  expect(settings().getByLabelText("Label")).toHaveValue("");
  expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
});

it("starts from the implicit step and shows a missing jump target", async () => {
  mockBackend(undefined, []);
  await renderEditor();
  expect(node("main")).toBeInTheDocument();
  mockBackend(undefined, [
    { id: "a", agent: {}, next: "gone", on_fail: "continue", extra: 1, wait: {} },
  ]);
  cleanup();
  await renderEditor();
  expect(settings().getByLabelText("Next")).toHaveValue("gone");
  expect(settings().getByLabelText("Type")).toHaveValue("");
  fireEvent.change(settings().getByLabelText("Type"), { target: { value: "command" } });
  expect(settings().getByLabelText("Type")).toHaveValue("command");
});

it("reports a definition it cannot read", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "automation_steps_source") throw "steps must be a list";
    return [];
  });
  render(<AutomationStepsEditor name="demo" onClose={vi.fn()} onSaved={vi.fn()} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("steps must be a list");
  expect(screen.getByRole("button", { name: "Add step" })).toBeDisabled();
});

it("opens from the pipeline tab and reloads the automation after a save", async () => {
  mockBackend();
  const onChanged = vi.fn();
  render(<AutomationPipeline name="demo" steps={[]} onChanged={onChanged} />);
  fireEvent.click(await screen.findByRole("button", { name: "Edit steps" }));
  await screen.findByRole("list", { name: "Draft steps" });
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  fireEvent.click(await screen.findByRole("button", { name: "Edit steps" }));
  await screen.findByRole("list", { name: "Draft steps" });
  fireEvent.change(settings().getByLabelText("Label"), { target: { value: "Build" } });
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save" })));
  expect(onChanged).toHaveBeenCalled();
  expect(await screen.findByRole("button", { name: "Edit steps" })).toBeInTheDocument();
});

it("keeps a call step as written and shows the automation it calls", async () => {
  const call: WrittenStep = {
    id: "delegate",
    call: { automation: "fix-issue", inputs: { issue: "{item.id}" } },
    on_fail: "block",
  };
  mockBackend(undefined, [STEPS[0], call]);
  await renderEditor();
  expect(node("delegate")).toHaveTextContent("fix-issue");
  fireEvent.click(node("delegate"));
  expect(settings().getByText(/called automation and its inputs/)).toBeInTheDocument();
  expect(settings().getByRole("option", { name: "call" })).toBeDisabled();
  fireEvent.change(settings().getByLabelText("Label"), { target: { value: "Delegate" } });
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save" })));

  expect(savedSteps()[1]).toEqual({ source: 1, value: { ...call, label: "Delegate" } });
});
