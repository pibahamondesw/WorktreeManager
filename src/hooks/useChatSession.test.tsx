// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { useChatSession } from "./useChatSession";
import { EMPTY_CONTROLS } from "../services/chat";

type LiveMessage = { generation: number; event: unknown };

const { invoke, channels } = vi.hoisted(() => ({
  invoke: vi.fn<typeof tauriInvoke>(),
  channels: [] as { onmessage: (message: LiveMessage) => void }[],
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  Channel: class {
    onmessage: (message: LiveMessage) => void = () => {};
    constructor() {
      channels.push(this);
    }
  },
}));

const opened = (sessionId: string, generation: number) => ({
  sessionId,
  generation,
  status: { kind: "idle" },
  items: [],
  pending: [],
  controls: EMPTY_CONTROLS,
});

const detaches = () => invoke.mock.calls.filter(([command]) => command === "chat_detach");

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  channels.length = 0;
});

describe("useChatSession", () => {
  it("detaches only the session it opened, after a restart opens another", async () => {
    const sessions = [opened("pane", 1), opened("pane-restarted", 2)];
    invoke.mockImplementation((command) =>
      Promise.resolve(command === "chat_open" ? sessions.shift() : undefined)
    );
    const view = renderHook(() => useChatSession("t1", "codex", ["/wt"]));
    await waitFor(() => expect(view.result.current.snapshot.generation).toBe(1));
    expect(view.result.current.snapshot).not.toHaveProperty("sessionId");

    await act(() => view.result.current.restart());
    await waitFor(() => expect(view.result.current.snapshot.generation).toBe(2));
    view.unmount();

    expect(detaches()).toEqual([
      ["chat_detach", { taskId: "t1", agent: "codex", generation: 2, sessionId: "pane-restarted" }],
    ]);
  });

  it("ignores events from the session it replaced", async () => {
    const sessions = [opened("pane", 1), opened("pane-restarted", 2)];
    invoke.mockImplementation((command) =>
      Promise.resolve(command === "chat_open" ? sessions.shift() : undefined)
    );
    const view = renderHook(() => useChatSession("t1", "codex", ["/wt"]));
    await waitFor(() => expect(view.result.current.snapshot.generation).toBe(1));
    await act(() => view.result.current.restart());
    await waitFor(() => expect(view.result.current.snapshot.generation).toBe(2));

    act(() =>
      channels[0].onmessage({ generation: 1, event: { type: "status", status: { kind: "busy" } } })
    );
    expect(view.result.current.snapshot.status).toEqual({ kind: "idle" });
    act(() =>
      channels[1].onmessage({ generation: 2, event: { type: "status", status: { kind: "busy" } } })
    );
    expect(view.result.current.snapshot.status).toEqual({ kind: "busy" });
  });
});
