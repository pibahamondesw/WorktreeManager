// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, renderHook } from "@testing-library/react";
import {
  nativeBindings,
  shortcutLabel,
  setShortcutCapture,
  setShortcutOverrides,
  useShortcutActions,
} from "./runtime";
import { shortcut } from "./catalog";
afterEach(async () => {
  cleanup();
  setShortcutOverrides({});
  await setShortcutCapture(false);
});

describe("shortcut routing", () => {
  it("replaces defaults and executes only one action across hook instances", () => {
    const action = vi.fn();
    const other = vi.fn();
    setShortcutOverrides({ "list.new": [shortcut("x")], "list.linear": [] });
    renderHook(() => useShortcutActions({ "list.new": { handler: action } }));
    const second = renderHook(() => useShortcutActions({ "list.linear": { handler: other } }));
    fireEvent.keyDown(window, { key: "n" });
    fireEvent.keyDown(window, { key: "l" });
    expect(action).not.toHaveBeenCalled();
    expect(other).not.toHaveBeenCalled();
    fireEvent.keyDown(window, { key: "x" });
    expect(action).toHaveBeenCalledTimes(1);
    second.unmount();
    fireEvent.keyDown(window, { key: "x" });
    expect(action).toHaveBeenCalledTimes(2);
  });
  it("resolves context before custom priority, including unavailable palette positions", () => {
    const workspace = vi.fn();
    const task = vi.fn();
    setShortcutOverrides({ "workspace.1": [shortcut("1", "meta")] });
    renderHook(() =>
      useShortcutActions({
        "workspace.1": { handler: workspace },
        "palette.jump.1": { handler: task, enabled: false },
      })
    );
    const modal = render(<div role="dialog" data-shortcut-context="palette" />);
    fireEvent.keyDown(window, { key: "1", metaKey: true });
    expect(workspace).not.toHaveBeenCalled();
    expect(task).not.toHaveBeenCalled();
    modal.unmount();
    fireEvent.keyDown(window, { key: "1", metaKey: true });
    expect(workspace).toHaveBeenCalledOnce();
  });
  it("blocks ambiguous customs and never falls back to defaults", () => {
    const first = vi.fn();
    const second = vi.fn();
    setShortcutOverrides({ "list.new": [shortcut("x")], "list.linear": [shortcut("x")] });
    renderHook(() =>
      useShortcutActions({ "list.new": { handler: first }, "list.linear": { handler: second } })
    );
    fireEvent.keyDown(window, { key: "x" });
    fireEvent.keyDown(window, { key: "n" });
    expect(first).not.toHaveBeenCalled();
    expect(second).not.toHaveBeenCalled();
  });
  it("gives a custom precedence over an unchanged default", () => {
    const first = vi.fn();
    const second = vi.fn();
    setShortcutOverrides({ "list.linear": [shortcut("n")] });
    renderHook(() =>
      useShortcutActions({ "list.new": { handler: first }, "list.linear": { handler: second } })
    );
    fireEvent.keyDown(window, { key: "n" });
    expect(second).toHaveBeenCalledOnce();
    expect(first).not.toHaveBeenCalled();
  });
  it("preserves typing and IME, but allows declared modified search shortcuts", () => {
    const create = vi.fn();
    const search = vi.fn();
    renderHook(() =>
      useShortcutActions({ "list.new": { handler: create }, "app.search": { handler: search } })
    );
    const view = render(<input aria-label="Text" />);
    const input = view.getByRole("textbox");
    fireEvent.keyDown(input, { key: "n" });
    fireEvent.keyDown(input, { key: "k", metaKey: true, isComposing: true });
    fireEvent.keyDown(input, { key: "k", metaKey: true });
    expect(create).not.toHaveBeenCalled();
    expect(search).toHaveBeenCalledOnce();
  });
  it("preserves button activation, modal isolation, repeat policy and capture", async () => {
    const open = vi.fn();
    const create = vi.fn();
    const zoom = vi.fn();
    renderHook(() =>
      useShortcutActions({
        "list.open": { handler: open },
        "list.new": { handler: create },
        "zoom.in": { handler: zoom },
      })
    );
    const view = render(<button>Open</button>);
    fireEvent.keyDown(view.getByRole("button"), { key: "Enter" });
    expect(open).not.toHaveBeenCalled();
    fireEvent.keyDown(window, { key: "n", repeat: true });
    expect(create).not.toHaveBeenCalled();
    fireEvent.keyDown(window, { key: "+", metaKey: true, repeat: true });
    expect(zoom).toHaveBeenCalledOnce();
    await setShortcutCapture(true);
    fireEvent.keyDown(window, { key: "n" });
    expect(create).not.toHaveBeenCalled();
    await setShortcutCapture(false);
    render(<div role="dialog" />);
    fireEvent.keyDown(window, { key: "n" });
    expect(create).not.toHaveBeenCalled();
  });
});

it("suppresses conflicting native accelerators only in overlapping contexts", () => {
  const config = { "palette.new": [shortcut("[", "meta")] };
  const native = nativeBindings(config).find((binding) => binding.id === "native.back.0")!;
  expect(native.contexts).not.toContain("palette");
  expect(native.contexts).toContain("task");
  expect(nativeBindings({ "native.back": [] }).some((binding) => binding.action === "back")).toBe(
    false
  );
});
it("uses event focus for chat shortcuts and preserves exact modifiers", () => {
  const action = vi.fn();
  renderHook(() => useShortcutActions({ "chat.model": { handler: action } }));
  const view = render(
    <div data-shortcut-context="chat">
      <textarea aria-label="Message" />
    </div>
  );
  const input = view.getByRole("textbox");
  fireEvent.keyDown(input, { key: "i", metaKey: true });
  fireEvent.keyDown(input, { key: "i", metaKey: true, shiftKey: true, altKey: true });
  expect(action).not.toHaveBeenCalled();
  fireEvent.keyDown(input, { key: "I", metaKey: true, shiftKey: true });
  expect(action).toHaveBeenCalledOnce();
});
it("supports Latin American bracket input without swallowing terminal typing", () => {
  const back = vi.fn();
  const sidebar = vi.fn();
  renderHook(() =>
    useShortcutActions({ "native.back": { handler: back }, "app.sidebar": { handler: sidebar } })
  );
  const view = render(
    <div className="xterm">
      <textarea aria-label="Terminal" />
    </div>
  );
  const input = view.getByRole("textbox");
  fireEvent.keyDown(input, { key: "[", shiftKey: true });
  expect(sidebar).not.toHaveBeenCalled();
  fireEvent.keyDown(input, { key: "[", metaKey: true, shiftKey: true });
  expect(back).toHaveBeenCalledOnce();
});

it("shows one preferred effective shortcut in app hints", () => {
  expect(shortcutLabel("app.settings", {})).toBe("⌘S");
  expect(shortcutLabel("workspace.1", {})).toBe("⌘1");
  expect(
    shortcutLabel("app.settings", { "app.settings": [{ key: "y", modifiers: ["ctrl"] }] })
  ).toBe("⌃Y");
  expect(shortcutLabel("app.settings", { "app.settings": [] })).toBe("");
});
