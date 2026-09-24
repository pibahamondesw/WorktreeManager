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

function setup(
  editorApp: "cursor" | "claude-code" | "codex" | "vscode-web",
  tasks: Task[] = [task]
) {
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

  it.each([
    ["claude-code", "claude"],
    ["codex", "codex"],
  ] as const)("opens the embedded surface for %s", async (editor, agent) => {
    const { result, showTask } = setup(editor);
    await act(() => result.current.openTask(task));
    expect(showTask).toHaveBeenCalledWith(task);
    expect(openEditorForWorktree).not.toHaveBeenCalled();
    expect(result.current.openedTask).toEqual({
      taskId: "t1",
      surface: { kind: "chat", agent },
    });
    act(() => result.current.closeTask());
    expect(result.current.openedTask).toBeNull();
  });

  it("opens an explicit agent in its remembered view and switches it in place", async () => {
    const recordTaskVisit = vi.fn();
    const { result } = renderHook(() =>
      useOpenTask({
        editorApp: "cursor",
        agentViews: { claude: "terminal", codex: "chat" },
        workspaces: [workspace],
        tasks: [task],
        recordTaskVisit,
        showTask: vi.fn(),
      })
    );
    await act(() => result.current.openTask(task, { agent: "claude" }));
    expect(openEditorForWorktree).not.toHaveBeenCalled();
    expect(result.current.openedTask).toEqual({
      taskId: "t1",
      surface: { kind: "terminal", agent: "claude" },
    });
    act(() => result.current.switchSurface("t1", { kind: "chat", agent: "claude" }));
    expect(result.current.openedTask?.surface).toEqual({ kind: "chat", agent: "claude" });
    act(() => result.current.switchSurface("other", { kind: "terminal", agent: "claude" }));
    expect(result.current.openedTask?.surface).toEqual({ kind: "chat", agent: "claude" });
  });

  it("closes the view when the task disappears", async () => {
    const { result, rerender } = setup("claude-code");
    await act(() => result.current.openTask(task));
    rerender({ tasks: [] });
    expect(result.current.openedTask).toBeNull();
  });
});
