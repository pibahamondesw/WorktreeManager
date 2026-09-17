// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAgentSessions } from "./useAgentSessions";

const mocks = vi.hoisted(() => ({ list: vi.fn(), onExit: vi.fn() }));
vi.mock("../services/terminal", () => ({
  terminalList: mocks.list,
  onTerminalExit: mocks.onExit,
}));
vi.mock("../services/codeEditor", () => ({
  editorList: vi.fn().mockResolvedValue([]),
  onEditorSession: vi.fn().mockResolvedValue(() => undefined),
}));

beforeEach(() => {
  mocks.list.mockReset();
  mocks.onExit.mockReset().mockResolvedValue(() => undefined);
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
