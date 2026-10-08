// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, renderHook, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import {
  isShortcutCaptureActive,
  nativeBindings,
  setShortcutCapture,
  syncNativeShortcuts,
  useShortcutActions,
} from "./runtime";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
beforeEach(() => {
  Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: {} });
  vi.mocked(invoke).mockResolvedValue(undefined);
});
afterEach(async () => {
  cleanup();
  await setShortcutCapture(false);
  Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  vi.resetAllMocks();
});
it("serializes capture, menu updates and restoration", async () => {
  const suspend = setShortcutCapture(true);
  const update = syncNativeShortcuts({ "native.back": [] });
  const restore = setShortcutCapture(false);
  await Promise.all([suspend, update, restore]);
  expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).toEqual([
    "set_shortcut_capture",
    "set_shortcut_menu",
    "set_shortcut_capture",
  ]);
  expect(invoke).toHaveBeenLastCalledWith("set_shortcut_capture", { active: false });
  expect(isShortcutCaptureActive()).toBe(false);
});
it("releases DOM capture on native suspension failure and permits retry", async () => {
  vi.mocked(invoke).mockRejectedValueOnce(new Error("menu unavailable"));
  await expect(setShortcutCapture(true)).rejects.toThrow("menu unavailable");
  expect(isShortcutCaptureActive()).toBe(false);
  await setShortcutCapture(true);
  expect(isShortcutCaptureActive()).toBe(true);
});
it("does not execute a native accelerator again through the DOM", () => {
  const back = vi.fn();
  renderHook(() => useShortcutActions({ "native.back": { handler: back } }));
  fireEvent.keyDown(window, { key: "[", metaKey: true });
  expect(back).not.toHaveBeenCalled();
});

it("registers Cmd+K as native app search for embedded-editor focus and follows rebinding", async () => {
  await syncNativeShortcuts({});
  const configured = nativeBindings({});
  expect(invoke).toHaveBeenCalledWith("set_shortcut_menu", { bindings: configured });
  const searchBinding = configured.find((binding) => binding.id === "app.search.0");
  expect(searchBinding).toMatchObject({ action: "search", key: "k", modifiers: ["meta"] });
  expect(searchBinding?.contexts).toContain("task");
  const rebound = nativeBindings({ "app.search": [{ key: "y", modifiers: ["meta"] }] });
  expect(rebound.find((binding) => binding.id === "app.search.0")?.key).toBe("y");
  expect(rebound.some((binding) => binding.action === "search" && binding.key === "k")).toBe(false);
  const search = vi.fn();
  renderHook(() => useShortcutActions({ "app.search": { handler: search } }));
  fireEvent.keyDown(window, { key: "k", metaKey: true });
  expect(search).not.toHaveBeenCalled();
});

it.each([
  ["app.history.back", "history-back", "ArrowLeft"],
  ["app.history.forward", "history-forward", "ArrowRight"],
])("routes %s through its native accelerator only", (id, action, key) => {
  const handler = vi.fn();
  renderHook(() => useShortcutActions({ [id]: { handler } }));
  expect(nativeBindings({}).find((binding) => binding.id === id + ".0")).toMatchObject({
    action,
    key,
    modifiers: ["meta"],
    inText: false,
  });
  fireEvent.keyDown(window, { key, metaKey: true });
  expect(handler).not.toHaveBeenCalled();
  expect(nativeBindings({ [id]: [] }).some((binding) => binding.action === action)).toBe(false);
});

it("updates native editing state when entering and leaving a text field", async () => {
  renderHook(() => useShortcutActions({}));
  const input = document.createElement("input");
  document.body.appendChild(input);
  input.focus();
  await waitFor(() =>
    expect(invoke).toHaveBeenLastCalledWith("set_shortcut_context", {
      context: "list",
      editing: true,
    })
  );
  input.blur();
  await waitFor(() =>
    expect(invoke).toHaveBeenLastCalledWith("set_shortcut_context", {
      context: "list",
      editing: false,
    })
  );
  input.remove();
});
