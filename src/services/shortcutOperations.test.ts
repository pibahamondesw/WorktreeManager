import { beforeEach, describe, expect, it, vi } from "vitest";
import { Operations } from "./operations";
import { DEFAULT_STATE, type AppState } from "../types";
import { shortcut } from "../shortcuts/catalog";
import { setShortcutOverrides, syncNativeShortcuts } from "../shortcuts/runtime";
vi.mock("../shortcuts/runtime", () => ({
  syncNativeShortcuts: vi.fn(),
  setShortcutOverrides: vi.fn(),
}));
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(syncNativeShortcuts).mockResolvedValue();
});
function harness(initial: AppState = DEFAULT_STATE) {
  let state = initial;
  const publish = vi.fn((next: AppState) => {
    state = next;
  });
  const save = vi.fn().mockResolvedValue(undefined);
  const operations = new Operations(
    () => state,
    publish,
    () => "cursor",
    save
  );
  return { operations, publish, save, state: () => state };
}
describe("shortcut operations", () => {
  it("persists reassignment in one write and publishes afterwards", async () => {
    const h = harness();
    await h.operations.updateShortcut("list.new", [shortcut("l")], true);
    expect(h.save).toHaveBeenCalledExactlyOnceWith([
      ["shortcutOverrides", { "list.linear": [], "list.new": [shortcut("l")] }],
    ]);
    expect(h.state().shortcutOverrides?.["list.linear"]).toEqual([]);
    expect(setShortcutOverrides).toHaveBeenCalledOnce();
  });
  it("rolls native bindings back and does not publish when persistence fails", async () => {
    const previous = { "list.new": [shortcut("x")] };
    const h = harness({ ...DEFAULT_STATE, shortcutOverrides: previous });
    h.save.mockRejectedValue(new Error("disk unavailable"));
    await expect(h.operations.updateShortcut("list.new", [shortcut("y")])).rejects.toThrow(
      "Could not save"
    );
    expect(h.publish).not.toHaveBeenCalled();
    expect(setShortcutOverrides).not.toHaveBeenCalled();
    expect(syncNativeShortcuts).toHaveBeenLastCalledWith(previous);
    expect(h.state().shortcutOverrides).toEqual(previous);
  });
  it("does not write if native validation fails or the binding is unchanged", async () => {
    const h = harness();
    await h.operations.updateShortcut("list.new", [shortcut("n")]);
    expect(h.save).not.toHaveBeenCalled();
    vi.mocked(syncNativeShortcuts).mockRejectedValue(new Error("Unsupported native key"));
    await expect(
      h.operations.updateShortcut("native.back", [shortcut("y", "meta")])
    ).rejects.toThrow("Unsupported");
    expect(h.save).not.toHaveBeenCalled();
    expect(h.publish).not.toHaveBeenCalled();
  });
  it("resets only shortcut overrides", async () => {
    const h = harness({ ...DEFAULT_STATE, shortcutOverrides: { "list.new": [] } });
    await h.operations.resetShortcuts();
    expect(h.save).toHaveBeenCalledExactlyOnceWith([["shortcutOverrides", {}]]);
    expect(h.state().vault).toEqual(DEFAULT_STATE.vault);
  });
});
