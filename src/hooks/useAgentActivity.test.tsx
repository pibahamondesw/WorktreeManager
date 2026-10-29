// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { useAgentActivity } from "./useAgentActivity";
import { Task } from "../types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("../embedded/sessionTransports", () => ({
  sessionRegistry: { onSessionEvent: () => () => {} },
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
});
