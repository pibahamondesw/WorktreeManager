import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));

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

describe("sessionRegistry.taskStatuses", () => {
  it.each([
    {
      status: { kind: "failed", message: "Unauthorized" },
      expected: { kind: "exited", code: null, hasError: true },
    },
    { status: { kind: "idle" }, hasError: true, expected: { kind: "running", hasError: true } },
  ])("preserves agent errors while other sessions run: %o", async ({ expected, ...chat }) => {
    invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "chat_list" ? [{ taskId: "t1", agent: "codex", ...chat }] : lists[command]
      )
    );
    const unsubscribe = sessionRegistry.subscribe(() => undefined);
    await vi.waitFor(() => expect(sessionRegistry.taskStatuses().t1).toEqual(expected));
    unsubscribe();
  });
});

describe("sessionRegistry.listLive", () => {
  it("keeps suspended surfaces available for task cleanup and surface switching", async () => {
    invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "opencode_list" ? [{ taskId: "sleeping", status: "suspended" }] : []
      )
    );
    await expect(sessionRegistry.listLive("sleeping")).resolves.toEqual([{ kind: "opencode" }]);
  });

  it("lists only the task's surfaces whose process is still alive", async () => {
    await expect(sessionRegistry.listLive("t1")).resolves.toEqual([
      { kind: "editor" },
      { kind: "chat", agent: "claude" },
      { kind: "terminal", agent: "codex" },
    ]);
  });
});

describe("sessionRegistry chat sessions", () => {
  it("follows only each task agent's primary session, not automations beside it", async () => {
    const chat = (sessionId: string, taskId: string, origin: string, status: unknown) => ({
      sessionId,
      taskId,
      agent: "codex",
      origin,
      status,
    });
    invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "chat_list"
          ? [
              chat("run", "t3", "automation", { kind: "busy" }),
              chat("pane", "t3", "user", { kind: "exited", code: null }),
              chat("other", "t4", "automation", { kind: "busy" }),
            ]
          : []
      )
    );
    await expect(sessionRegistry.listLive("t3")).resolves.toEqual([]);
    await expect(sessionRegistry.listLive("t4")).resolves.toEqual([]);
    const unsubscribe = sessionRegistry.subscribe(() => undefined);
    await vi.waitFor(() =>
      expect(sessionRegistry.taskStatuses().t3).toEqual({ kind: "exited", code: null })
    );
    expect(sessionRegistry.taskStatuses().t4).toBeUndefined();
    unsubscribe();
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
