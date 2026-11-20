// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, renderHook } from "@testing-library/react";
import { useShortcutActions } from "./runtime";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(() => Promise.resolve()) }));
vi.mock("../platform", () => ({ IS_WINDOWS: true }));

beforeEach(() => {
  Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: {} });
});
afterEach(() => {
  cleanup();
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
});

it("runs native accelerators through the DOM on Windows, which has no app menu", () => {
  const back = vi.fn();
  renderHook(() => useShortcutActions({ "native.back": { handler: back } }));
  fireEvent.keyDown(window, { key: "[", ctrlKey: true });
  expect(back).toHaveBeenCalledOnce();
});
