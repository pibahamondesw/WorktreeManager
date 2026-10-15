// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { onChatStatus } from "../services/chat";
import type { onTerminalExit } from "../services/terminal";
import type { onOpenCodeSession } from "../services/opencode";
import { useAgentSessions } from "./useAgentSessions";

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  onExit: vi.fn<typeof onTerminalExit>(),
  chatList: vi.fn(),
  onChatStatus: vi.fn<typeof onChatStatus>(),
  openCodeList: vi.fn(),
  onOpenCodeSession: vi.fn<typeof onOpenCodeSession>(),
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
vi.mock("../services/opencode", () => ({
  openCodeList: mocks.openCodeList,
  onOpenCodeSession: mocks.onOpenCodeSession,
}));

beforeEach(() => {
  mocks.list.mockReset();
  mocks.onExit.mockReset().mockResolvedValue(() => undefined);
  mocks.chatList.mockReset().mockResolvedValue([]);
  mocks.onChatStatus.mockReset().mockResolvedValue(() => undefined);
  mocks.openCodeList.mockReset().mockResolvedValue([]);
  mocks.onOpenCodeSession.mockReset().mockResolvedValue(() => undefined);
});

it("counts a task's OpenCode as running until its server ends", async () => {
  mocks.list.mockResolvedValue([]);
  const running = { taskId: "t1", generation: "g", status: "running", pid: 1, error: null };
  mocks.openCodeList.mockResolvedValue([running]);
  const { result } = renderHook(() => useAgentSessions(false));
  await waitFor(() => expect(result.current.t1).toEqual({ kind: "running" }));
  const onSession = mocks.onOpenCodeSession.mock.calls[0][0];
  void act(() => onSession({ ...running, status: "exited", error: "ended" }));
  expect(result.current.t1).toEqual({ kind: "exited", code: null });
});

it("counts a live chat as a running session and follows its status events", async () => {
  mocks.list.mockResolvedValue([]);
  mocks.chatList.mockResolvedValue([{ taskId: "t1", agent: "codex", status: { kind: "busy" } }]);
  const { result } = renderHook(() => useAgentSessions(false));
  await waitFor(() => expect(result.current.t1).toEqual({ kind: "running" }));
  const onStatus = mocks.onChatStatus.mock.calls[0][0];
  void act(() =>
    onStatus({
      taskId: "t1",
      agent: "codex",
      status: { kind: "failed", message: "x" },
      waiting: false,
    })
  );
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
    void act(() => onExit({ taskId: "t1", agent: "codex", code: 1 }));
    expect(result.current.t1).toEqual({ kind: "running" });
    void act(() => onExit({ taskId: "t1", agent: "claude", code: 0 }));
    expect(result.current.t1.kind).toBe("exited");
  }
);

it("keeps loaded sessions when terminal exit subscription fails", async () => {
  mocks.list.mockResolvedValue([{ taskId: "t1", agent: "codex", status: { kind: "running" } }]);
  mocks.onExit.mockRejectedValueOnce(new Error("Events unavailable"));
  const { result } = renderHook(() => useAgentSessions(false));
  await waitFor(() => expect(result.current.t1).toEqual({ kind: "running" }));
});
