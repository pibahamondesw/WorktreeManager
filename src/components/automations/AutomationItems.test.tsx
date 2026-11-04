// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { invoke } from "@tauri-apps/api/core";
import { AutomationDetail } from "./AutomationDetail";
import { OpenConversationContext } from "./openConversation";
import { AutomationTasksContext } from "./automationTasks";
import {
  AutomationState,
  AutomationSummary,
  PipelineStep,
  RunItem,
} from "../../services/automations";
import { Task } from "../../types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

const STEPS: PipelineStep[] = [
  {
    id: "implement",
    label: "implementing",
    next: null,
    onFail: "block",
    kind: {
      agent: {
        agent: { id: "claude", model: "opus", effort: null },
        skill: null,
        prompt: "Fix",
        onReject: null,
      },
    },
  },
  {
    id: "verify",
    label: "verifying",
    next: null,
    onFail: "block",
    kind: { command: { command: ["make", "check"], repo: "app" } },
  },
];

const state = (overrides: AutomationState["overrides"] = {}): AutomationState => ({
  enabled: true,
  pause_reason: null,
  next_run_at: null,
  consecutive_failures: 0,
  overrides,
});

const summary = (overrides: AutomationState["overrides"] = {}): AutomationSummary => ({
  entry: {
    name: "rubocop",
    path: "/automations/rubocop/AUTOMATION.md",
    definition: {
      name: "rubocop",
      description: null,
      trigger: { manual: true, schedule: null },
      steps: STEPS,
      selector: { maxNewItems: 2 },
      pools: { active: 1, openPrs: 3 },
    },
    errors: [],
    warnings: [],
  },
  state: state(overrides),
  stateError: null,
  latestRun: null,
});

const item = (id: string, overrides: Partial<RunItem> = {}): RunItem => ({
  id,
  version: "1",
  title: null,
  status: "queued",
  phase: null,
  steps: [],
  run_id: "r1",
  test: false,
  ...overrides,
});

const ITEMS: RunItem[] = [
  item("cop-a", { title: "Style/A", status: "active", phase: "implement", task_id: "t-a" }),
  item("cop-b", { title: "Style/B", status: "active", phase: "implement" }),
  item("cop-c", {
    title: "Style/C",
    status: "blocked",
    phase: "verify",
    cause: "budget:repair_rounds",
    task_id: "t-c",
    steps: [
      {
        step: "implement",
        attempt: 1,
        status: "succeeded",
        process_end: null,
        exit_code: null,
        outcome: null,
        stage: null,
        verdict: null,
        cause: null,
        head: null,
        session_id: "s1",
        conversation_id: "conv-c",
      },
      {
        step: "verify",
        attempt: 2,
        status: "failed",
        process_end: "exited",
        exit_code: 1,
        outcome: "failure",
        stage: null,
        verdict: null,
        cause: "make check exited 1",
        head: null,
        session_id: null,
        conversation_id: null,
      },
    ],
  }),
];

const TASKS: Task[] = [
  {
    id: "t-a",
    workspaceId: "w1",
    branchName: "pedro/wor-1-style-a",
    linearIssueIdentifier: "WOR-1",
    members: [],
    createdAt: "2026-11-01T00:00:00Z",
  },
  {
    id: "t-c",
    workspaceId: "w1",
    branchName: "pedro/wor-3-style-c",
    members: [],
    createdAt: "2026-11-01T00:00:00Z",
  },
];

const openConversation = vi.fn(() => true);
const onChanged = vi.fn();

function answer(results: Record<string, unknown>) {
  vi.mocked(invoke).mockImplementation((command: string) =>
    command in results
      ? Promise.resolve(results[command])
      : Promise.reject(new Error(`unexpected ${command}`))
  );
}

async function renderDetail(current = summary()) {
  render(
    <OpenConversationContext.Provider value={openConversation}>
      <AutomationTasksContext.Provider value={TASKS}>
        <AutomationDetail summary={current} editorApp="cursor" onChanged={onChanged} />
      </AutomationTasksContext.Provider>
    </OpenConversationContext.Provider>
  );
  await act(async () => {});
}

const column = (label: string) => screen.getByRole("region", { name: label });

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(invoke).mockReset();
  openConversation.mockReset().mockReturnValue(true);
  onChanged.mockReset();
});

it("opens campaigns on a board of their items by phase", async () => {
  answer({ automation_items_list: ITEMS });
  await renderDetail();

  expect(screen.getByRole("tab", { name: "Items" })).toHaveAttribute("aria-selected", "true");
  expect(invoke).toHaveBeenCalledWith("automation_items_list", { name: "rubocop" });
  const implementing = within(column("implementing"));
  expect(implementing.getByText("Style/A")).toBeInTheDocument();
  expect(implementing.getByText("Style/B")).toBeInTheDocument();
  expect(implementing.getByText("WOR-1")).toBeInTheDocument();
  expect(implementing.getByText("pedro/wor-1-style-a")).toBeInTheDocument();
  const blocked = within(column("Blocked"));
  expect(blocked.getByText("budget:repair rounds")).toHaveClass("select-text");
  expect(blocked.getByText(/verifying #2 \(failed\): make check exited 1/)).toHaveClass(
    "select-text"
  );
  expect(within(column("Queued")).queryAllByRole("article")).toHaveLength(0);
});

it("moves between cards with the arrow keys and opens the focused item with Enter", async () => {
  answer({ automation_items_list: ITEMS });
  await renderDetail();
  const first = screen.getByRole("article", { name: "Style/A" });
  expect(first).toHaveAttribute("tabindex", "0");
  expect(screen.getByRole("article", { name: "Style/B" })).toHaveAttribute("tabindex", "-1");

  first.focus();
  fireEvent.keyDown(first, { key: "ArrowDown" });
  expect(screen.getByRole("article", { name: "Style/B" })).toHaveFocus();
  fireEvent.keyDown(document.activeElement!, { key: "ArrowRight" });
  const blocked = screen.getByRole("article", { name: "Style/C" });
  expect(blocked).toHaveFocus();
  expect(blocked).toHaveAttribute("tabindex", "0");
  fireEvent.keyDown(blocked, { key: "ArrowRight" });
  expect(blocked).toHaveFocus();

  fireEvent.keyDown(blocked, { key: "Enter" });
  expect(openConversation).toHaveBeenCalledWith({
    taskId: "t-c",
    agent: "claude",
    conversationId: "conv-c",
  });
  fireEvent.keyDown(blocked, { key: "ArrowLeft" });
  fireEvent.keyDown(document.activeElement!, { key: "ArrowUp" });
  expect(first).toHaveFocus();

  expect(
    within(screen.getByRole("article", { name: "Style/B" })).queryByRole("button")
  ).not.toBeInTheDocument();
});

it("shows items whose task was deleted as removed instead of offering to open them", async () => {
  answer({
    automation_items_list: [
      item("cop-d", { title: "Style/D", status: "completed", task_id: "t-gone" }),
    ],
  });
  await renderDetail();

  const card = within(screen.getByRole("article", { name: "Style/D" }));
  expect(card.getByText("Task removed")).toBeInTheDocument();
  expect(card.queryByRole("button")).not.toBeInTheDocument();
  fireEvent.keyDown(screen.getByRole("article", { name: "Style/D" }), { key: "Enter" });
  expect(openConversation).not.toHaveBeenCalled();
});

it("grants one more item with Next and goes back to Continuous", async () => {
  const granted = { value: { mode: "next", remaining: 1 }, by: "user", at: "2026-11-04T10:00:00Z" };
  answer({
    automation_items_list: [],
    automation_next: {
      state: state(),
      run: { run_id: "r9", status: "starting", skip_reason: null },
    },
  });
  await renderDetail();
  expect(screen.getByText("Continuous", { selector: "span" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Continuous" })).toBeDisabled();
  expect(screen.getByText("No items yet")).toBeInTheDocument();

  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Next" })));

  expect(invoke).toHaveBeenCalledWith("automation_next", { name: "rubocop", by: "user" });
  expect(screen.getByText("One more item granted; run r9 started")).toBeInTheDocument();
  expect(onChanged).toHaveBeenCalled();

  cleanup();
  answer({
    automation_items_list: [],
    automation_next: {
      state: state(),
      run: { run_id: "r10", status: "skipped", skip_reason: "already_running" },
    },
    automation_continuous: state(),
  });
  await renderDetail(summary({ pace: granted as never }));
  expect(screen.getByText("Next · 1 item granted")).toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Next" })));
  expect(
    screen.getByText("One more item granted; run skipped: already running")
  ).toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Continuous" })));
  expect(invoke).toHaveBeenCalledWith("automation_continuous", { name: "rubocop", by: "user" });

  cleanup();
  answer({ automation_items_list: [], automation_next: { state: state(), run: null } });
  await renderDetail();
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Next" })));
  expect(screen.getByText("One more item granted for the next run")).toBeInTheDocument();
});

it("edits the limits live and resets them to the definition", async () => {
  const override = { value: 4, by: "cli" as const, at: "2026-11-04T10:00:00Z" };
  answer({ automation_items_list: [], automation_set_limit: state() });
  await renderDetail(summary({ max_open_prs: override }));
  const maxActive = screen.getByLabelText("Max active");
  expect(maxActive).toHaveValue(1);
  expect(screen.getByLabelText("Max open PRs")).toHaveValue(4);
  expect(screen.getByText(/^Set by the CLI on/)).toHaveClass("select-text");

  fireEvent.change(maxActive, { target: { value: "0" } });
  expect(screen.queryByRole("button", { name: "Set" })).not.toBeInTheDocument();
  fireEvent.keyDown(maxActive, { key: "Enter" });
  expect(invoke).not.toHaveBeenCalledWith("automation_set_limit", expect.anything());
  fireEvent.change(maxActive, { target: { value: "3" } });
  fireEvent.keyDown(maxActive, { key: "Escape" });
  expect(maxActive).toHaveValue(1);
  fireEvent.change(maxActive, { target: { value: "3" } });
  await act(async () => fireEvent.keyDown(maxActive, { key: "Enter" }));
  expect(invoke).toHaveBeenCalledWith("automation_set_limit", {
    name: "rubocop",
    by: "user",
    limit: "max_active",
    value: 3,
  });

  fireEvent.change(screen.getByLabelText("Batch size"), { target: { value: "5" } });
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Set" })));
  expect(invoke).toHaveBeenCalledWith("automation_set_limit", {
    name: "rubocop",
    by: "user",
    limit: "batch_size",
    value: 5,
  });

  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Reset" })));
  expect(invoke).toHaveBeenCalledWith("automation_set_limit", {
    name: "rubocop",
    by: "user",
    limit: "max_open_prs",
    value: null,
  });
});

it("shows why a verb failed and where the items could not be read", async () => {
  vi.mocked(invoke).mockImplementation((command: string) =>
    Promise.reject(command === "automation_next" ? "demo is not a campaign" : "No items")
  );
  await renderDetail();
  expect(screen.getByText("No items")).toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Next" })));
  expect(screen.getByText("demo is not a campaign")).toHaveClass("select-text");
});

it("refreshes the board while an item is active", async () => {
  vi.useFakeTimers();
  try {
    answer({ automation_items_list: ITEMS });
    await renderDetail();
    const calls = vi.mocked(invoke).mock.calls.length;
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(vi.mocked(invoke).mock.calls.length).toBeGreaterThan(calls);
  } finally {
    vi.useRealTimers();
  }
});
