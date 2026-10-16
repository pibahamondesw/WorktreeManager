import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { listLiveSurfaces, stopTaskSurface } from "./surfaceLifecycle";

const lists: Record<string, unknown[]> = {
  vscode_list: [
    { taskId: "t1", status: "running" },
    { taskId: "t2", status: "running" },
  ],
  opencode_list: [{ taskId: "t1", status: "exited" }],
  chat_list: [
    { taskId: "t1", agent: "claude", status: { kind: "busy" } },
    { taskId: "t1", agent: "codex", status: { kind: "failed", message: "boom" } },
  ],
  terminal_list: [
    { taskId: "t1", agent: "codex", status: { kind: "running" } },
    { taskId: "t1", agent: "claude", status: { kind: "exited", code: 0 } },
  ],
};

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation((command: string) => Promise.resolve(lists[command]));
});

describe("listLiveSurfaces", () => {
  it("lists only the task's surfaces whose process is still alive", async () => {
    await expect(listLiveSurfaces("t1")).resolves.toEqual([
      { kind: "editor" },
      { kind: "chat", agent: "claude" },
      { kind: "terminal", agent: "codex" },
    ]);
  });
});

describe("listLiveSurfaces with a failing list", () => {
  it("still reports what the other lists know", async () => {
    invoke.mockImplementation((command: string) =>
      command === "opencode_list"
        ? Promise.reject(new Error("down"))
        : Promise.resolve(lists[command])
    );
    await expect(listLiveSurfaces("t2")).resolves.toEqual([{ kind: "editor" }]);
  });
});

describe("stopTaskSurface", () => {
  it.each([
    [{ kind: "editor" }, "vscode_close", { taskId: "t1" }],
    [{ kind: "opencode" }, "opencode_close", { taskId: "t1" }],
    [{ kind: "chat", agent: "claude" }, "chat_stop", { taskId: "t1", agent: "claude" }],
    [{ kind: "terminal", agent: "codex" }, "terminal_stop", { taskId: "t1", agent: "codex" }],
  ] as const)("closes %o through its own close command", async (surface, command, args) => {
    await stopTaskSurface("t1", surface);
    expect(invoke).toHaveBeenCalledWith(command, args);
  });

  it("has nothing to close for an external editor", async () => {
    await stopTaskSurface("t1", { kind: "external" });
    expect(invoke).not.toHaveBeenCalled();
  });
});
