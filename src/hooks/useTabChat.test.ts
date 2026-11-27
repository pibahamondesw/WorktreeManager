// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { ChatInfo, chatList, onChatStatus } from "../services/chat";
import { useTabChat } from "./useTabChat";

vi.mock("../services/chat", async (original) => ({
  ...(await original<typeof import("../services/chat")>()),
  chatList: vi.fn(),
  onChatStatus: vi.fn(),
}));
afterEach(cleanup);

const info = (extra: Partial<ChatInfo>): ChatInfo => ({
  sessionId: "s1",
  taskId: "t2",
  agent: "codex",
  origin: "user",
  status: { kind: "busy" },
  waiting: false,
  backgroundAgents: false,
  ...extra,
});

it("follows the owner's chat in the tab, ignoring others", async () => {
  let report: (chat: ChatInfo) => void = () => undefined;
  const stop = vi.fn();
  vi.mocked(onChatStatus).mockImplementation((callback) => {
    report = callback;
    return Promise.resolve(stop);
  });
  vi.mocked(chatList).mockResolvedValue([info({ tab: "a1" }), info({ tab: "a2" })]);
  const { result, unmount } = renderHook(() => useTabChat("t2", "codex", "a1"));
  await waitFor(() => expect(result.current?.status).toEqual({ kind: "busy" }));
  act(() => {
    report(info({ tab: "a1", origin: "automation", status: { kind: "idle" } }));
    report(info({ tab: "a1", agent: "claude", status: { kind: "idle" } }));
    report(info({ taskId: "t3", tab: "a1", status: { kind: "idle" } }));
  });
  expect(result.current?.status).toEqual({ kind: "busy" });
  act(() => report(info({ tab: "a1", status: { kind: "idle" } })));
  expect(result.current?.status).toEqual({ kind: "idle" });
  unmount();
  await waitFor(() => expect(stop).toHaveBeenCalled());
});

it("reads the main tab's chat and tolerates an unreadable list", async () => {
  vi.mocked(onChatStatus).mockResolvedValue(vi.fn());
  vi.mocked(chatList).mockClear();
  vi.mocked(chatList)
    .mockRejectedValueOnce("down")
    .mockResolvedValueOnce([info({})]);
  const { result, rerender } = renderHook(({ tab }) => useTabChat("t2", "codex", tab), {
    initialProps: { tab: "a1" },
  });
  await waitFor(() => expect(chatList).toHaveBeenCalledTimes(1));
  expect(result.current).toBeNull();
  rerender({ tab: "main" });
  await waitFor(() => expect(result.current?.sessionId).toBe("s1"));
});
