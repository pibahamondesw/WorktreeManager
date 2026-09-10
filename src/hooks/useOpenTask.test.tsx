// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";

const openEditorForWorktree = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
vi.mock("../services/openEditor", () => ({ openEditorForWorktree }));

import { useOpenTask } from "./useOpenTask";
import { Task, Workspace } from "../types";

const workspace = { id: "ws1", name: "Payments", repos: [] } as unknown as Workspace;
const task = {
  id: "t1",
  workspaceId: "ws1",
  branchName: "feat/x",
  members: [{ path: "/wt/a" }, { path: "/wt/b" }],
} as unknown as Task;

function setup(editorApp: "cursor" | "claude-code" | "vscode-web", tasks: Task[] = [task]) {
  const recordTaskVisit = vi.fn();
  const showTask = vi.fn();
  const hook = renderHook(
    ({ tasks }) =>
      useOpenTask({ editorApp, workspaces: [workspace], tasks, recordTaskVisit, showTask }),
    { initialProps: { tasks } }
  );
  return { ...hook, recordTaskVisit, showTask };
}

beforeEach(() => openEditorForWorktree.mockClear());

describe("useOpenTask", () => {
  it("restores a task from history without recording another visit or opening an external editor", async () => {
    const second = { ...task, id: "t2" };
    const { result, recordTaskVisit } = setup("vscode-web", [task, second]);
    await act(() => result.current.openTask(second));
    act(() => result.current.restoreTask(task));
    expect(result.current.openedTask?.taskId).toBe(task.id);
    expect(recordTaskVisit).toHaveBeenCalledTimes(1);
    expect(openEditorForWorktree).not.toHaveBeenCalled();
  });
  it("switches embedded editors by task without launching an external workspace", async () => {
    const second = { ...task, id: "t2" };
    const { result } = setup("vscode-web", [task, second]);
    await act(() => result.current.openTask(task));
    expect(result.current.openedTask).toEqual({ taskId: "t1", surface: { kind: "editor" } });
    await act(() => result.current.openTask(second));
    expect(result.current.openedTask).toEqual({ taskId: "t2", surface: { kind: "editor" } });
    expect(openEditorForWorktree).not.toHaveBeenCalled();
  });
  it("launches the external editor and keeps nothing open", async () => {
    const { result, recordTaskVisit } = setup("cursor");
    await act(() => result.current.openTask(task, { onError: () => undefined }));
    expect(recordTaskVisit).toHaveBeenCalledWith(task);
    expect(openEditorForWorktree).toHaveBeenCalledWith(
      "cursor",
      ["/wt/a", "/wt/b"],
      "feat/x",
      "Payments",
      expect.any(Object)
    );
    expect(result.current.openedTask).toBeNull();
  });

  it("opens the embedded surface for the claude-code editor", async () => {
    const { result, showTask } = setup("claude-code");
    await act(() => result.current.openTask(task));
    expect(showTask).toHaveBeenCalledWith(task);
    expect(openEditorForWorktree).not.toHaveBeenCalled();
    expect(result.current.openedTask).toEqual({
      taskId: "t1",
      surface: { kind: "terminal", agent: "claude" },
    });
    act(() => result.current.closeTask());
    expect(result.current.openedTask).toBeNull();
  });

  it("lets an explicit surface override the editor choice", async () => {
    const { result } = setup("cursor");
    await act(() =>
      result.current.openTask(task, { surface: { kind: "terminal", agent: "claude" } })
    );
    expect(openEditorForWorktree).not.toHaveBeenCalled();
    expect(result.current.openedTask?.taskId).toBe("t1");
  });

  it("closes the view when the task disappears", async () => {
    const { result, rerender } = setup("claude-code");
    await act(() => result.current.openTask(task));
    rerender({ tasks: [] });
    expect(result.current.openedTask).toBeNull();
  });
});
