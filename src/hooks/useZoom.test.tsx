// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useKeyboardShortcuts } from "./useKeyboardShortcuts";
import { ZoomControls } from "../components/ui/ZoomControls";

beforeEach(() => localStorage.clear());
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function press(key: string, modifiers: KeyboardEventInit = {}) {
  return fireEvent.keyDown(window, { key, metaKey: true, ...modifiers });
}

describe("interface zoom", () => {
  it("handles plus with and without Shift, equals and minus", () => {
    render(<ZoomControls />);
    expect(document.documentElement.style.fontSize).toBe("16px");
    expect(press("+", { shiftKey: true })).toBe(false);
    press("+");
    press("=");
    expect(screen.getByRole("status").textContent).toBe("130%");
    press("-");
    expect(screen.getByRole("status").textContent).toBe("120%");
    expect(document.documentElement.style.fontSize).toBe("19.2px");
  });

  it("works in text fields without intercepting workspace navigation", () => {
    const selectFirstWorkspace = vi.fn();
    function WorkspaceShortcuts() {
      useKeyboardShortcuts({ "meta+0": { handler: selectFirstWorkspace } });
      return null;
    }
    render(
      <>
        <WorkspaceShortcuts />
        <ZoomControls />
        <input aria-label="Search" />
      </>
    );
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "+", ctrlKey: true });
    expect(screen.getByRole("status").textContent).toBe("110%");
    press("0");
    expect(selectFirstWorkspace).toHaveBeenCalledOnce();
    expect(screen.getByRole("status").textContent).toBe("110%");
  });

  it("ignores unmodified typing, Alt combinations and composition", () => {
    render(<ZoomControls />);
    press("+", { metaKey: false });
    press("-", { altKey: true });
    press("=", { isComposing: true });
    expect(screen.getByRole("status").textContent).toBe("100%");
  });

  it("clamps repeated shortcuts and supports mouse controls", () => {
    render(<ZoomControls />);
    for (let i = 0; i < 20; i++) press("+");
    expect(screen.getByRole("status").textContent).toBe("150%");
    expect((screen.getByRole("button", { name: "Zoom in" }) as HTMLButtonElement).disabled).toBe(
      true
    );
    for (let i = 0; i < 20; i++) press("-");
    expect(screen.getByRole("status").textContent).toBe("80%");
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(screen.getByRole("status").textContent).toBe("90%");
    fireEvent.click(screen.getByRole("button", { name: /Reset zoom/ }));
    expect(screen.getByRole("status").textContent).toBe("100%");
  });

  it("restores the preference after remounting and cleans up styles and listeners", () => {
    const { unmount } = render(<ZoomControls />);
    press("+");
    unmount();
    expect(document.documentElement.style.fontSize).toBe("");
    expect(press("+")).toBe(true);
    render(<ZoomControls />);
    expect(screen.getByRole("status").textContent).toBe("110%");
  });

  it.each(["garbage", "Infinity", "0", "200", "90.5"])("rejects invalid saved zoom %s", (value) => {
    localStorage.setItem("worktreemanager.uiZoom", value);
    render(<ZoomControls />);
    expect(screen.getByRole("status").textContent).toBe("100%");
  });

  it("remains usable when storage throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("unavailable");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("unavailable");
    });
    render(<ZoomControls />);
    press("+");
    expect(screen.getByRole("status").textContent).toBe("110%");
  });
});
