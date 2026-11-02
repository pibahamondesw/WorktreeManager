// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AutomationRun, AutomationSummary, listAutomationRuns } from "../services/automations";
import { loadAutomationInbox, persist } from "../services/store";
import { useAutomationInbox } from "./useAutomationInbox";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("../services/automations", async (original) => ({
  ...(await original<typeof import("../services/automations")>()),
  listAutomationRuns: vi.fn(),
}));
vi.mock("../services/store", () => ({
  loadAutomationInbox: vi.fn(),
  persist: vi.fn().mockResolvedValue(undefined),
}));

const summary = (name: string): AutomationSummary =>
  ({
    entry: { name, path: "", definition: null, errors: [], warnings: [] },
    state: null,
    stateError: null,
    latestRun: null,
  }) as AutomationSummary;

const failed = (id: string, task_id: string | null = "t1"): AutomationRun => ({
  id,
  trigger: null,
  status: "failed",
  skip_reason: null,
  failure_reason: "step_failed",
  outcome: null,
  claimed_at_ms: 1,
  ended_at_ms: Number(id),
  items: [],
  task_id,
});

const notifications = () =>
  vi.mocked(invoke).mock.calls.filter(([command]) => command === "agent_attention");

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(invoke).mockClear();
  vi.mocked(persist).mockClear();
  vi.mocked(listen).mockClear();
  vi.mocked(loadAutomationInbox).mockResolvedValue({ read: ["run:demo:1"], archived: [] });
});

function renderInbox(runs: AutomationRun[], onOpenEntry = vi.fn()) {
  vi.mocked(listAutomationRuns).mockImplementation(async (name) =>
    name === "broken" ? Promise.reject("unreadable") : { runs, next_before: null }
  );
  return renderHook(({ automations }) => useAutomationInbox(automations, onOpenEntry), {
    initialProps: { automations: [summary("demo"), summary("broken")] },
  });
}

it("notifies entries that arrive after the first load and keeps marks in app data", async () => {
  const { result, rerender } = renderInbox([failed("2"), failed("1")]);
  await waitFor(() => expect(result.current.items).toHaveLength(2));
  expect(result.current.unread).toBe(1);
  expect(result.current.badge).toBe(1);
  expect(notifications()).toEqual([]);

  vi.mocked(listAutomationRuns).mockResolvedValue({
    runs: [failed("3", null), failed("2"), failed("1")],
    next_before: null,
  });
  rerender({ automations: [summary("demo")] });
  await waitFor(() => expect(notifications()).toHaveLength(1));
  expect(notifications()[0][1]).toEqual({
    sound: null,
    notification: {
      title: "demo",
      body: "Failed: step failed",
      target: { taskId: "", surface: "chat", agent: "", inboxEntry: "run:demo:3" },
      taskOpen: false,
    },
  });

  act(() => result.current.setArchived(["run:demo:2"], true));
  act(() => result.current.setRead(["run:demo:3"], true));
  expect(persist).toHaveBeenLastCalledWith([
    ["automationInbox", { read: ["run:demo:1", "run:demo:3"], archived: ["run:demo:2"] }],
  ]);
  expect(result.current.unread).toBe(0);
});

it("opens the entry of a clicked notification and marks it read", async () => {
  const onOpenEntry = vi.fn();
  const { result } = renderInbox([failed("2")], onOpenEntry);
  await waitFor(() => expect(result.current.unread).toBe(1));
  const [, open] = vi
    .mocked(listen)
    .mock.calls.filter(([event]) => event === "agent-notification-open")
    .at(-1)!;

  act(() => open({ payload: { taskId: "t1", inboxEntry: "run:demo:9" } } as never));
  expect(onOpenEntry).not.toHaveBeenCalled();
  act(() => open({ payload: { taskId: "t1", inboxEntry: "run:demo:2" } } as never));

  expect(onOpenEntry).toHaveBeenCalledWith(expect.objectContaining({ key: "run:demo:2" }));
  expect(result.current.unread).toBe(0);
});

it("starts with no marks when they cannot be read", async () => {
  vi.mocked(loadAutomationInbox).mockRejectedValue(new Error("store"));
  const { result } = renderInbox([failed("1")]);
  await waitFor(() => expect(result.current.unread).toBe(1));
});
