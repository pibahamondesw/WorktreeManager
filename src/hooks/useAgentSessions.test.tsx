// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAgentSessions } from "./useAgentSessions";

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  onExit: vi.fn(),
  chatList: vi.fn(),
  onChatStatus: vi.fn(),
}));
vi.mock("../services/terminal", () => ({
  terminalList: mocks.list,
  onTerminalExit: mocks.onExit,
}));
vi.mock("../services/chat", () => ({
  chatList: mocks.chatList,
  onChatStatus: mocks.onChatStatus,
  isLive: (status: { kind: string }) => status.kind !== "exited" && status.kind !== "failed",
}));
vi.mock("../services/codeEditor", () => ({
  editorList: vi.fn().mockResolvedValue([]),
  onEditorSession: vi.fn().mockResolvedValue(() => undefined),
}));

beforeEach(() => {
  mocks.list.mockReset();
  mocks.onExit.mockReset().mockResolvedValue(() => undefined);
  mocks.chatList.mockReset().mockResolvedValue([]);
  mocks.onChatStatus.mockReset().mockResolvedValue(() => undefined);
});

it("counts a live chat as a running session and follows its status events", async () => {
  mocks.list.mockResolvedValue([]);
  mocks.chatList.mockResolvedValue([{ taskId: "t1", agent: "codex", status: { kind: "busy" } }]);
  const { result } = renderHook(() => useAgentSessions(false));
  await waitFor(() => expect(result.current.t1).toEqual({ kind: "running" }));
  const onStatus = mocks.onChatStatus.mock.calls[0][0];
  act(() => onStatus({ taskId: "t1", agent: "codex", status: { kind: "failed", message: "x" } }));
  expect(result.current.t1).toEqual({ kind: "exited", code: null });
});
afterEach(cleanup);

it.each([true, false])(
  "keeps a task running while either agent is alive (reverse: %s)",
  async (reverse) => {
    const sessions = [
      { taskId: "t1", agent: "claude", status: { kind: "running" } },
      { taskId: "t1", agent: "codex", status: { kind: "exited", code: 0 } },
    ];
    mocks.list.mockResolvedValue(reverse ? sessions.reverse() : sessions);
    const { result } = renderHook(() => useAgentSessions(false));
    await waitFor(() => expect(result.current.t1).toEqual({ kind: "running" }));
    const onExit = mocks.onExit.mock.calls[0][0];
    act(() => onExit({ taskId: "t1", agent: "codex", code: 1 }));
    expect(result.current.t1).toEqual({ kind: "running" });
    act(() => onExit({ taskId: "t1", agent: "claude", code: 0 }));
    expect(result.current.t1.kind).toBe("exited");
  }
);
