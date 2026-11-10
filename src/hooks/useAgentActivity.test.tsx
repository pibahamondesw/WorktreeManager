// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useAgentActivity } from "./useAgentActivity";
import { Task } from "../types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
const chatListeners = vi.hoisted(() => new Set<(chat: unknown, previous?: unknown) => void>());
vi.mock("../embedded/sessionTransports", () => ({
  sessionRegistry: {
    onSessionEvent: (_kind: string, listener: (chat: unknown, previous?: unknown) => void) => {
      chatListeners.add(listener);
      return () => chatListeners.delete(listener);
    },
  },
}));
vi.mock("../services/store", () => ({
  loadAgentAlerts: vi.fn().mockRejectedValue(new Error("unused")),
  persist: vi.fn().mockResolvedValue(undefined),
}));

afterEach(() => {
  cleanup();
  vi.mocked(invoke).mockClear();
});

const task: Task = {
  id: "t1",
  workspaceId: "w1",
  branchName: "branch",
  createdAt: "2026-10-29T00:00:00Z",
  members: [],
};

const surfaceTurnCalls = () =>
  vi
    .mocked(invoke)
    .mock.calls.filter(([command]) => command === "set_surface_turn_in_progress")
    .map(([, args]) => args);

describe("useAgentActivity", () => {
  it("tells the backend when an embedded terminal turn starts and ends", async () => {
    const { result } = renderHook(() =>
      useAgentActivity({
        tasks: [task],
        sessions: {},
        openedTaskId: null,
        onOpenTarget: vi.fn(),
      })
    );
    await waitFor(() => expect(surfaceTurnCalls()).toEqual([{ inProgress: false }]));

    act(() => {
      result.current.handleAgentEvent({
        state: "working",
        agent: "claude",
        surface: "terminal",
        taskId: "t1",
        at: 1,
      });
    });
    await waitFor(() => expect(surfaceTurnCalls().at(-1)).toEqual({ inProgress: true }));

    act(() => {
      result.current.handleAgentEvent({
        state: "done",
        agent: "claude",
        surface: "terminal",
        taskId: "t1",
        at: 2,
      });
    });
    await waitFor(() => expect(surfaceTurnCalls().at(-1)).toEqual({ inProgress: false }));
  });

  it("adds unread Inbox entries to the Dock badge and leaves Inbox notification clicks alone", async () => {
    const onOpenTarget = vi.fn();
    const { rerender } = renderHook(
      ({ inboxBadge }) =>
        useAgentActivity({
          tasks: [task],
          sessions: {},
          openedTaskId: null,
          onOpenTarget,
          inboxBadge,
        }),
      { initialProps: { inboxBadge: 2 } }
    );
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("agent_badge", { count: 2 }));
    rerender({ inboxBadge: 0 });
    await waitFor(() => expect(invoke).toHaveBeenLastCalledWith("agent_badge", { count: 0 }));

    const [, open] = vi
      .mocked(listen)
      .mock.calls.filter(([event]) => event === "agent-notification-open")
      .at(-1)!;
    const target = { taskId: "t1", surface: "chat", agent: "claude" };
    act(() => open({ payload: { ...target, inboxEntry: "run:demo:1" } } as never));
    expect(onOpenTarget).not.toHaveBeenCalled();
    act(() => open({ payload: target } as never));
    expect(onOpenTarget).toHaveBeenCalledWith(task, target);
  });

  it("reports finished and waiting agents to the automation engine, except automation sessions", async () => {
    const { result } = renderHook(() =>
      useAgentActivity({ tasks: [task], sessions: {}, openedTaskId: null, onOpenTarget: vi.fn() })
    );
    const appEvents = () =>
      vi
        .mocked(invoke)
        .mock.calls.filter(([command]) => command === "automation_app_event")
        .map(([, args]) => (args as { event: unknown }).event);
    const report = (state: "working" | "done", at: number) =>
      act(() => {
        result.current.handleAgentEvent({
          state,
          agent: "claude",
          surface: "terminal",
          taskId: "t1",
          at,
        });
      });
    report("working", 1);
    report("done", 2);
    report("done", 3);
    const chat = {
      sessionId: "s1",
      taskId: "t1",
      agent: "claude",
      origin: "automation",
      status: { kind: "idle" },
      waiting: true,
      backgroundAgents: false,
    };
    const now = vi.spyOn(Date, "now").mockReturnValue(10);
    act(() => chatListeners.forEach((listener) => listener(chat, undefined)));
    act(() =>
      chatListeners.forEach((listener) => listener({ ...chat, sessionId: "s2", origin: "user" }))
    );
    now.mockRestore();

    await waitFor(() => expect(appEvents()).toHaveLength(2));
    const eventTask = { id: "t1", workspaceId: "w1", branch: "branch" };
    expect(appEvents()).toEqual([
      {
        type: "agent.done",
        task: eventTask,
        agent: { id: "claude", surface: "terminal", at: 2 },
      },
      {
        type: "agent.waiting",
        task: eventTask,
        agent: { id: "claude", surface: "chat", at: 10 },
      },
    ]);
  });
});
