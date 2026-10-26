// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";

const openEditorForWorktree = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
vi.mock("../services/openEditor", () => ({ openEditorForWorktree }));
const surfaces = vi.hoisted(() => ({
  listLive: vi.fn(),
  stop: vi.fn(),
}));
vi.mock("../embedded/sessionTransports", () => ({ sessionRegistry: surfaces }));

import { useOpenTask } from "./useOpenTask";
import { Task, TaskSurface } from "../types";

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
    ({ tasks }) => useOpenTask({ editorApp, tasks, recordTaskVisit, showTask }),
    { initialProps: { tasks } }
  );
  return { ...hook, recordTaskVisit, showTask };
}

function deferred<Value>() {
  let resolve!: (value: Value) => void;
  const promise = new Promise<Value>((settle) => (resolve = settle));
  return { promise, resolve };
}

const live = (...list: TaskSurface[]) => surfaces.listLive.mockResolvedValue(list);

beforeEach(() => {
  openEditorForWorktree.mockClear();
  surfaces.listLive.mockReset();
  surfaces.stop.mockReset();
  live();
  surfaces.stop.mockResolvedValue(undefined);
});

describe("useOpenTask", () => {
  it("restores a task from history without recording another visit or opening an external editor", async () => {
    const second = { ...task, id: "t2" };
    const { result, recordTaskVisit } = setup("vscode-web", [task, second]);
    await act(() => result.current.openTask(second));
    act(() => result.current.restoreTask(task));
    await waitFor(() => expect(result.current.openedTask?.taskId).toBe(task.id));
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
      "t1",
      undefined,
      expect.any(Object)
    );
    expect(result.current.openedTask).toBeNull();
  });

  it("passes the persisted legacy workspace path when opening an older task", async () => {
    const legacy = { ...task, workspaceFilePath: "/workspaces/payments/feat-x.code-workspace" };
    const { result } = setup("cursor", [legacy]);
    await act(() => result.current.openTask(legacy));
    expect(openEditorForWorktree).toHaveBeenCalledWith(
      "cursor",
      ["/wt/a", "/wt/b"],
      legacy.id,
      legacy.workspaceFilePath,
      expect.any(Object)
    );
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

  describe("when the task already has a different live surface", () => {
    const pendingOpen = async (
      result: { current: ReturnType<typeof useOpenTask> },
      options = {}
    ) => {
      let opened!: Promise<boolean>;
      act(() => {
        opened = result.current.openTask(task, options);
      });
      await waitFor(() => expect(result.current.surfaceSwitch).not.toBeNull());
      return { opened };
    };

    it("asks which surface to use and keeps the live one by default", async () => {
      live({ kind: "editor" });
      const { result } = setup("claude-code");
      const { opened } = await pendingOpen(result);
      expect(result.current.surfaceSwitch).toEqual({
        taskId: "t1",
        current: { kind: "editor" },
        requested: { kind: "chat", agent: "claude" },
        closing: [{ kind: "editor" }],
        phase: "asking",
      });
      act(() => result.current.resolveSurfaceSwitch("keep"));
      await act(async () => {
        await expect(opened).resolves.toBe(true);
      });
      expect(result.current.openedTask).toEqual({ taskId: "t1", surface: { kind: "editor" } });
      expect(result.current.surfaceSwitch).toBeNull();
      expect(surfaces.stop).not.toHaveBeenCalled();
    });

    it("closes the live surfaces before opening the requested one", async () => {
      live({ kind: "editor" }, { kind: "terminal", agent: "codex" });
      const { result } = setup("claude-code");
      const { opened } = await pendingOpen(result);
      act(() => result.current.resolveSurfaceSwitch("replace"));
      await act(async () => {
        await expect(opened).resolves.toBe(true);
      });
      expect(surfaces.stop.mock.calls).toEqual([
        ["t1", { kind: "editor" }],
        ["t1", { kind: "terminal", agent: "codex" }],
      ]);
      expect(result.current.openedTask).toEqual({
        taskId: "t1",
        surface: { kind: "chat", agent: "claude" },
      });
    });

    it("does nothing when cancelled", async () => {
      live({ kind: "opencode" });
      const { result, recordTaskVisit } = setup("vscode-web");
      const { opened } = await pendingOpen(result);
      act(() => result.current.resolveSurfaceSwitch("cancel"));
      await expect(opened).resolves.toBe(false);
      expect(result.current.openedTask).toBeNull();
      expect(recordTaskVisit).not.toHaveBeenCalled();
    });

    it("keeps asking with the error and the surfaces still open when closing fails", async () => {
      live({ kind: "editor" }, { kind: "opencode" });
      surfaces.stop.mockResolvedValueOnce(undefined).mockRejectedValueOnce("server busy");
      const { result } = setup("claude-code");
      const { opened } = await pendingOpen(result);
      act(() => result.current.resolveSurfaceSwitch("replace"));
      expect(result.current.surfaceSwitch?.phase).toBe("closing");
      await waitFor(() =>
        expect(result.current.surfaceSwitch).toMatchObject({
          phase: "asking",
          closing: [{ kind: "opencode" }],
          error: "Could not close OpenCode: server busy",
        })
      );
      expect(result.current.openedTask).toBeNull();
      act(() => result.current.resolveSurfaceSwitch("replace"));
      await act(async () => {
        await expect(opened).resolves.toBe(true);
      });
      expect(surfaces.stop).toHaveBeenLastCalledWith("t1", { kind: "opencode" });
      expect(result.current.surfaceSwitch).toBeNull();
      expect(result.current.openedTask?.surface).toEqual({ kind: "chat", agent: "claude" });
    });

    it("drops a replacement that a newer navigation overtook while closing", async () => {
      live({ kind: "opencode" });
      const stop = deferred<undefined>();
      surfaces.stop.mockReturnValue(stop.promise);
      const second = { ...task, id: "t2" };
      const { result } = setup("vscode-web", [task, second]);
      const { opened } = await pendingOpen(result);
      act(() => result.current.resolveSurfaceSwitch("replace"));
      live();
      await act(() => result.current.openTask(second));
      await act(async () => {
        stop.resolve(undefined);
        await expect(opened).resolves.toBe(false);
      });
      expect(result.current.openedTask).toEqual({ taskId: "t2", surface: { kind: "editor" } });
    });

    it("cancels the question when a newer navigation takes over", async () => {
      live({ kind: "opencode" });
      const { result } = setup("vscode-web");
      const { opened } = await pendingOpen(result);
      act(() => result.current.openTaskSurface(task, { kind: "opencode" }));
      await expect(opened).resolves.toBe(false);
      await waitFor(() =>
        expect(result.current.openedTask).toEqual({ taskId: "t1", surface: { kind: "opencode" } })
      );
      expect(result.current.surfaceSwitch).toBeNull();
    });

    it("asks before a notification reopens a surface that was replaced", async () => {
      live({ kind: "editor" });
      const { result } = setup("vscode-web");
      act(() => result.current.openTaskSurface(task, { kind: "chat", agent: "claude" }));
      await waitFor(() =>
        expect(result.current.surfaceSwitch).toMatchObject({
          current: { kind: "editor" },
          requested: { kind: "chat", agent: "claude" },
        })
      );
      act(() => result.current.resolveSurfaceSwitch("keep"));
      await waitFor(() =>
        expect(result.current.openedTask).toEqual({ taskId: "t1", surface: { kind: "editor" } })
      );
    });

    it("asks when another agent is requested explicitly", async () => {
      live({ kind: "chat", agent: "claude" });
      const { result } = setup("cursor");
      await pendingOpen(result, { agent: "codex" });
      expect(result.current.surfaceSwitch?.requested).toEqual({ kind: "chat", agent: "codex" });
    });

    it("cancels the question when its task is deleted", async () => {
      live({ kind: "opencode" });
      const { result, rerender } = setup("vscode-web");
      const { opened } = await pendingOpen(result);
      rerender({ tasks: [] });
      await expect(opened).resolves.toBe(false);
      expect(result.current.surfaceSwitch).toBeNull();
      expect(surfaces.stop).not.toHaveBeenCalled();
    });

    it.each(["vscode-web", "cursor"] as const)(
      "restores history onto the live surface without asking (selector %s)",
      async (editor) => {
        live({ kind: "chat", agent: "codex" });
        const { result } = setup(editor);
        act(() => result.current.restoreTask(task));
        await waitFor(() =>
          expect(result.current.openedTask).toEqual({
            taskId: "t1",
            surface: { kind: "chat", agent: "codex" },
          })
        );
        expect(result.current.surfaceSwitch).toBeNull();
      }
    );

    it("ignores a restore that a later close superseded", async () => {
      const list = deferred<TaskSurface[]>();
      surfaces.listLive.mockReturnValue(list.promise);
      const { result, showTask } = setup("vscode-web");
      act(() => result.current.restoreTask(task));
      act(() => result.current.closeTask());
      await act(async () => {
        list.resolve([{ kind: "editor" }]);
        await list.promise;
      });
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(showTask).not.toHaveBeenCalled();
      expect(result.current.openedTask).toBeNull();
    });

    it("opens an external editor without asking or closing anything", async () => {
      live({ kind: "editor" });
      const { result } = setup("cursor");
      await act(() => result.current.openTask(task, { onError: () => undefined }));
      expect(openEditorForWorktree).toHaveBeenCalled();
      expect(result.current.surfaceSwitch).toBeNull();
      expect(surfaces.listLive).not.toHaveBeenCalled();
    });

    it("treats an unreadable session list as nothing live", async () => {
      surfaces.listLive.mockRejectedValue(new Error("ipc down"));
      const { result } = setup("vscode-web");
      await act(() => result.current.openTask(task));
      expect(result.current.openedTask).toEqual({ taskId: "t1", surface: { kind: "editor" } });
    });
  });
});
