// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { readDiffLayout, saveDiffLayout } from "./diffLayoutPreference";

afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

it("reads unknown stored values as the unified layout", () => {
  localStorage.setItem("worktreemanager.diffLayout", "sideways");
  expect(readDiffLayout()).toBe("unified");
});

it("falls back to the unified layout when storage is unavailable", () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
    throw new Error("denied");
  });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new Error("denied");
  });

  expect(() => saveDiffLayout("split")).not.toThrow();
  expect(readDiffLayout()).toBe("unified");
});
