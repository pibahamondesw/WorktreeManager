// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useNow } from "./useNow";

afterEach(() => vi.useRealTimers());

it("refreshes the current time on each interval", () => {
  vi.useFakeTimers({ now: 1_000 });
  const { result } = renderHook(() => useNow(60_000));
  expect(result.current).toBe(1_000);
  act(() => {
    vi.advanceTimersByTime(60_000);
  });
  expect(result.current).toBe(61_000);
});
