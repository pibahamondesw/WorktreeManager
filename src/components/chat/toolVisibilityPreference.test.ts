// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import {
  readToolVisibility,
  saveToolVisibility,
  useToolVisibility,
} from "./toolVisibilityPreference";

afterEach(() => {
  vi.restoreAllMocks();
  saveToolVisibility("all");
  localStorage.clear();
});

it("reads unknown stored values as showing every tool call", () => {
  localStorage.setItem("worktreemanager.toolVisibility", "sideways");
  expect(readToolVisibility()).toBe("all");
});

it("stores the choice and switches every chat using it", () => {
  const first = renderHook(() => useToolVisibility());
  const second = renderHook(() => useToolVisibility());
  act(() => saveToolVisibility("zen"));
  expect(first.result.current).toBe("zen");
  expect(second.result.current).toBe("zen");
  expect(readToolVisibility()).toBe("zen");
});

it("still switches when storage is unavailable", () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
    throw new Error("denied");
  });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new Error("denied");
  });
  const { result } = renderHook(() => useToolVisibility());

  expect(() => act(() => saveToolVisibility("hideAll"))).not.toThrow();
  expect(result.current).toBe("hideAll");
  expect(readToolVisibility()).toBe("all");
});
