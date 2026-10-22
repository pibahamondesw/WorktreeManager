// @vitest-environment jsdom
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useTaskProjects } from "./useTaskProjects";
import { Operations } from "../services/operations";
import { LinearService } from "../services/linear";
import { taskLinearStore } from "../services/taskLinear";
import { AppState, DEFAULT_STATE, Task } from "../types";

beforeEach(() => {
  taskLinearStore.reset();
  vi.useFakeTimers();
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

it("persists shared project updates without refetching on metadata changes and refreshes on the shared interval", async () => {
  const fetch = vi.spyOn(LinearService.prototype, "fetchIssueLinearInfoBatch").mockResolvedValue({
    issue: { status: null, prs: [], project: { id: "p1", name: "Payments" } },
    "new-issue": { status: null, prs: [], project: { id: "p2", name: "API" } },
  });
  const workspaces = [
    { id: "configured", name: "API", repos: [], linearApiKey: "test-key" },
    { id: "manual", name: "Manual", repos: [] },
  ];
  const task: Task = {
    id: "task",
    workspaceId: "configured",
    linearIssueId: "issue",
    branchName: "branch",
    createdAt: "2026-01-01",
    members: [],
  };
  let state: AppState = { ...DEFAULT_STATE, tasks: [task], workspaces };
  const save = vi.fn().mockResolvedValue(undefined);
  const operations = new Operations(
    () => state,
    (next) => {
      state = next;
    },
    () => "cursor",
    save
  );
  const { rerender, unmount } = renderHook(
    ({ tasks }) => useTaskProjects(tasks, workspaces, operations),
    { initialProps: { tasks: state.tasks } }
  );
  await act(async () => {
    await Promise.resolve();
  });
  expect(state.tasks[0].linearProjectName).toBe("Payments");
  expect(fetch).toHaveBeenCalledExactlyOnceWith(["issue"]);
  rerender({ tasks: state.tasks });
  await act(async () => {
    await Promise.resolve();
  });
  expect(fetch).toHaveBeenCalledOnce();
  expect(save).toHaveBeenCalledOnce();
  state = { ...state, tasks: [{ ...task, linearIssueId: "new-issue" }] };
  rerender({ tasks: state.tasks });
  await act(async () => {
    await Promise.resolve();
  });
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(state.tasks[0].linearProjectName).toBe("API");
  await act(() => vi.advanceTimersByTimeAsync(15 * 60 * 1000));
  expect(fetch).toHaveBeenCalledTimes(3);
  unmount();
  await act(() => vi.advanceTimersByTimeAsync(15 * 60 * 1000));
  expect(fetch).toHaveBeenCalledTimes(3);
});
