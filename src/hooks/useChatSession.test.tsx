// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { useChatSession } from "./useChatSession";
import { EMPTY_CONTROLS, PendingRequest } from "../services/chat";

type LiveMessage = { generation: number; events: unknown[] };

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
  it("keeps elicitation answers bound to the task and generation that displayed them", async () => {
    const sessions = [opened("first", 1), opened("second", 2)];
    invoke.mockImplementation((command) =>
      Promise.resolve(command === "chat_open" ? sessions.shift() : undefined)
    );
    const view = renderHook(({ taskId }) => useChatSession(taskId, "codex", ["/wt"]), {
      initialProps: { taskId: "first" },
    });
    await waitFor(() => expect(view.result.current.snapshot.generation).toBe(1));
    const firstRespond = view.result.current.respond;
    view.rerender({ taskId: "second" });
    await waitFor(() => expect(view.result.current.snapshot.generation).toBe(2));
    const response = { elicitation: { action: "cancel" as const, content: null } };
    await firstRespond("elicitation:1", response);
    expect(invoke).toHaveBeenCalledWith("chat_respond", {
      taskId: "first",
      agent: "codex",
      generation: 1,
      requestId: "elicitation:1",
      response,
    });
    await view.result.current.respond("elicitation:1", response);
    expect(invoke).toHaveBeenLastCalledWith("chat_respond", {
      taskId: "second",
      agent: "codex",
      generation: 2,
      requestId: "elicitation:1",
      response,
    });
  });

  it("removes elicitation controls on external resolution without sending an answer", async () => {
    const pending: PendingRequest = {
      id: "elicitation:1",
      kind: "elicitation",
      title: "MCP",
      format: "text",
      serverName: "browser",
      mode: "url",
      url: "https://example.com",
      elicitationId: "login",
    };
    invoke.mockImplementation((command) =>
      Promise.resolve(
        command === "chat_open" ? { ...opened("pane", 1), pending: [pending] } : undefined
      )
    );
    const view = renderHook(() => useChatSession("t1", "codex", ["/wt"]));
    await waitFor(() => expect(view.result.current.snapshot.pending).toHaveLength(1));
    act(() =>
      channels[0].onmessage({ generation: 1, events: [{ type: "resolved", id: pending.id }] })
    );
    expect(view.result.current.snapshot.pending).toEqual([]);
    expect(invoke.mock.calls.some(([command]) => command === "chat_respond")).toBe(false);
  });

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
      channels[0].onmessage({
        generation: 1,
        events: [{ type: "status", status: { kind: "busy" } }],
      })
    );
    expect(view.result.current.snapshot.status).toEqual({ kind: "idle" });
    act(() =>
      channels[1].onmessage({
        generation: 2,
        events: [{ type: "status", status: { kind: "busy" } }],
      })
    );
    expect(view.result.current.snapshot.status).toEqual({ kind: "busy" });
  });
});
