import { afterEach, expect, it, vi } from "vitest";
import { onChatIntentRequest, requestChatIntent, takeRequestedChatIntent } from "./chatIntents";

afterEach(() => vi.useRealTimers());

const intent = { agent: "claude", mode: "restart" as const, conversation: "c1" };

it("hands a requested intent once, to its own agent, and tells listeners of the task", () => {
  const listener = vi.fn();
  const other = vi.fn();
  const stop = onChatIntentRequest("t1", listener);
  const stopOther = onChatIntentRequest("t2", other);
  requestChatIntent("t1", intent);
  expect(listener).toHaveBeenCalledWith(intent);
  expect(other).not.toHaveBeenCalled();
  expect(takeRequestedChatIntent("t1", "codex")).toBeNull();
  expect(takeRequestedChatIntent("t1", "claude")).toEqual(intent);
  expect(takeRequestedChatIntent("t1", "claude")).toBeNull();
  stop();
  stopOther();
  requestChatIntent("t1", intent);
  expect(listener).toHaveBeenCalledTimes(1);
  expect(takeRequestedChatIntent("t1", "claude")).toEqual(intent);
});

it("drops a request no chat took in time", () => {
  vi.useFakeTimers();
  requestChatIntent("t1", intent);
  vi.advanceTimersByTime(30_001);
  expect(takeRequestedChatIntent("t1", "claude")).toBeNull();
});
