import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke, channels } = vi.hoisted(() => {
  const channels: { onmessage?: (e: unknown) => void }[] = [];
  return { invoke: vi.fn(), channels };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  Channel: class {
    onmessage?: (e: unknown) => void;
    constructor() {
      channels.push(this);
    }
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { terminalClose, terminalOpen } from "./terminal";

beforeEach(() => {
  invoke.mockReset();
  channels.length = 0;
});

describe("terminalOpen", () => {
  it("passes the agent and routes channel messages to onEvent", async () => {
    invoke.mockResolvedValue({ created: true, status: { kind: "running" }, replay: "" });
    const onEvent = vi.fn();
    await terminalOpen({
      taskId: "t1",
      agent: "claude",
      folders: ["/wt"],
      branchName: "feat/x",
      cols: 80,
      rows: 24,
      onEvent,
    });
    const [command, args] = invoke.mock.calls[0];
    expect(command).toBe("terminal_open");
    expect(args).toMatchObject({ taskId: "t1", agent: "claude", folders: ["/wt"], cols: 80 });
    channels[0].onmessage?.({ type: "data", data: "hi" });
    expect(onEvent).toHaveBeenCalledWith({ type: "data", data: "hi" });
  });
});

describe("terminalClose", () => {
  it("swallows rejections", async () => {
    invoke.mockRejectedValue("no terminal for task");
    await expect(terminalClose("t1")).resolves.toBeUndefined();
  });
});
