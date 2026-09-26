// @vitest-environment jsdom
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useTaskProjects } from "./useTaskProjects";
import { Operations } from "../services/operations";
import { DEFAULT_STATE, Task } from "../types";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("refreshes configured workspaces on mount, issue changes and interval without looping on metadata updates", () => {
  vi.useFakeTimers();
  const operations = new Operations(
    () => DEFAULT_STATE,
    vi.fn(),
    () => "cursor"
  );
  const refresh = vi.spyOn(operations, "refreshTaskProjects").mockResolvedValue(undefined);
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
  const { rerender, unmount } = renderHook(
    ({ tasks }) => useTaskProjects(tasks, workspaces, operations),
    { initialProps: { tasks: [task] } }
  );
  expect(refresh).toHaveBeenCalledExactlyOnceWith("configured");
  rerender({ tasks: [{ ...task, linearProjectName: "Payments" }] });
  expect(refresh).toHaveBeenCalledTimes(1);
  rerender({ tasks: [{ ...task, linearIssueId: "new-issue" }] });
  expect(refresh).toHaveBeenCalledTimes(2);
  act(() => vi.advanceTimersByTime(15 * 60 * 1000));
  expect(refresh).toHaveBeenCalledTimes(3);
  unmount();
  act(() => vi.advanceTimersByTime(15 * 60 * 1000));
  expect(refresh).toHaveBeenCalledTimes(3);
});
