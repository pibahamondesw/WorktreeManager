import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { sessionRegistry } from "./sessionTransports";

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

describe("sessionRegistry.listLive", () => {
  it("lists only the task's surfaces whose process is still alive", async () => {
    await expect(sessionRegistry.listLive("t1")).resolves.toEqual([
      { kind: "editor" },
      { kind: "chat", agent: "claude" },
      { kind: "terminal", agent: "codex" },
    ]);
  });
});

describe("sessionRegistry.listLive with a failing list", () => {
  it("still reports what the other lists know", async () => {
    invoke.mockImplementation((command: string) =>
      command === "opencode_list"
        ? Promise.reject(new Error("down"))
        : Promise.resolve(lists[command])
    );
    await expect(sessionRegistry.listLive("t2")).resolves.toEqual([{ kind: "editor" }]);
  });
});

describe("sessionRegistry.stop", () => {
  it.each([
    [{ kind: "editor" }, "vscode_close", { taskId: "t1" }],
    [{ kind: "opencode" }, "opencode_close", { taskId: "t1" }],
    [{ kind: "chat", agent: "claude" }, "chat_stop", { taskId: "t1", agent: "claude" }],
    [{ kind: "terminal", agent: "codex" }, "terminal_stop", { taskId: "t1", agent: "codex" }],
  ] as const)("closes %o through its own close command", async (surface, command, args) => {
    await sessionRegistry.stop("t1", surface);
    expect(invoke).toHaveBeenCalledWith(command, args);
  });

  it("has nothing to close for an external editor", async () => {
    await sessionRegistry.stop("t1", { kind: "external" });
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("sessionRegistry.closeAll", () => {
  it("closes the task through every transport's close command", async () => {
    await sessionRegistry.closeAll("t1");
    expect(invoke.mock.calls.map(([command]) => command as string).sort()).toEqual([
      "chat_close",
      "opencode_close",
      "terminal_close",
      "vscode_close",
    ]);
    expect(
      invoke.mock.calls.every(([, args]) => (args as { taskId: string }).taskId === "t1")
    ).toBe(true);
  });
});
