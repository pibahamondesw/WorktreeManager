// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, renderHook } from "@testing-library/react";
import { useKeyboardShortcuts } from "./useKeyboardShortcuts";

afterEach(() => {
  cleanup();
  document.body.innerHTML = "";
});

function mountXterm() {
  const host = document.createElement("div");
  host.className = "xterm";
  const helper = document.createElement("textarea");
  host.appendChild(helper);
  const surface = document.createElement("div");
  host.appendChild(surface);
  document.body.appendChild(host);
  return { helper, surface };
}

const press = (target: Element, key: string, init: KeyboardEventInit = {}) =>
  target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, ...init }));

describe("useKeyboardShortcuts inside a terminal", () => {
  it("lets bare keys reach the terminal but fires meta combos marked inTextFields", () => {
    const bracket = vi.fn();
    const j = vi.fn();
    const back = vi.fn();
    renderHook(() =>
      useKeyboardShortcuts({
        "[": { handler: bracket },
        j: { handler: j },
        "meta+[": { handler: back, inTextFields: true },
      })
    );
    const { helper, surface } = mountXterm();
    press(helper, "[");
    press(surface, "j");
    expect(bracket).not.toHaveBeenCalled();
    expect(j).not.toHaveBeenCalled();
    press(surface, "[", { metaKey: true });
    expect(back).toHaveBeenCalledTimes(1);
    press(document.body, "[");
    expect(bracket).toHaveBeenCalledTimes(1);
  });
});
