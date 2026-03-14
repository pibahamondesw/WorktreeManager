import { describe, it, expect } from "vitest";
import { parseKey } from "./useKeyboardShortcuts";

describe("parseKey", () => {
  it("parses a plain key", () => {
    expect(parseKey("n")).toEqual({ key: "n", meta: false, shift: false });
  });

  it("parses meta+key", () => {
    expect(parseKey("meta+b")).toEqual({ key: "b", meta: true, shift: false });
  });

  it("parses meta+shift+key", () => {
    expect(parseKey("meta+shift+c")).toEqual({ key: "c", meta: true, shift: true });
  });

  it("parses special keys like ArrowDown", () => {
    expect(parseKey("ArrowDown")).toEqual({ key: "ArrowDown", meta: false, shift: false });
  });

  it("parses Enter", () => {
    expect(parseKey("Enter")).toEqual({ key: "Enter", meta: false, shift: false });
  });

  it("parses Escape", () => {
    expect(parseKey("Escape")).toEqual({ key: "Escape", meta: false, shift: false });
  });

  it("parses numeric keys", () => {
    expect(parseKey("1")).toEqual({ key: "1", meta: false, shift: false });
  });

  it("handles shift without meta", () => {
    expect(parseKey("shift+a")).toEqual({ key: "a", meta: false, shift: true });
  });
});
