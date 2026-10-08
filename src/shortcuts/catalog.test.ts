import { describe, expect, it } from "vitest";
import {
  bindingById,
  bindings,
  conflicts,
  effectiveBindings,
  normalizeOverrides,
  overlaps,
  shortcut,
  updateBinding,
} from "./catalog";

const entry = (id: string) => bindingById.get(id)!;
describe("personal bindings", () => {
  it("keeps defaults in code and distinguishes reset from disable", () => {
    expect(effectiveBindings(entry("list.new"), {})).toEqual([shortcut("n")]);
    expect(updateBinding({}, "list.new", [])).toEqual({ "list.new": [] });
    expect(updateBinding({ "list.new": [] }, "list.new", null)).toEqual({});
    expect(updateBinding({}, "list.new", [shortcut("n")])).toEqual({});
  });
  it("removes the old alternatives when assigning a personal binding", () => {
    const custom = updateBinding({}, "app.settings", [shortcut("s", "meta", "shift")]);
    expect(effectiveBindings(entry("app.settings"), custom)).toEqual([
      shortcut("s", "shift", "meta"),
    ]);
  });
  it("requires confirmation and only removes the colliding alternative", () => {
    expect(() => updateBinding({}, "list.new", [shortcut("b", "meta")])).toThrow(
      "Confirm reassignment"
    );
    const next = updateBinding({}, "list.new", [shortcut("b", "meta")], true);
    expect(next["list.branch"]).toEqual([shortcut("b", "ctrl")]);
    const disabled = updateBinding({}, "list.new", [shortcut("l")], true);
    expect(disabled["list.linear"]).toEqual([]);
    expect(effectiveBindings(entry("list.linear"), disabled)).toEqual([]);
  });
  it("checks restore collisions and protected navigation", () => {
    const overrides = { "list.new": [], "list.linear": [shortcut("n")] };
    expect(() => updateBinding(overrides, "list.new", null)).toThrow("Confirm reassignment");
    expect(() => updateBinding({}, "list.new", [shortcut("Enter")], true)).toThrow("Reserved");
    expect(() => updateBinding({}, "list.open", [])).toThrow("reserved");
  });
  it("allows reuse across exclusive contexts and reserves disabled positional actions", () => {
    expect(conflicts(entry("palette.jump.1"), [shortcut("1", "meta")], {})).toEqual([]);
    expect(conflicts(entry("list.new"), [shortcut("9", "meta")], {}).map((v) => v.id)).toContain(
      "workspace.9"
    );
  });
  it("matches symbols consistently without treating shifted digits as equal", () => {
    expect(overlaps(shortcut("[", "meta"), shortcut("[", "meta", "shift"))).toBe(true);
    expect(overlaps(shortcut("1", "meta"), shortcut("1", "meta", "shift"))).toBe(false);
    expect(overlaps(shortcut("k", "meta"), shortcut("k", "ctrl"))).toBe(false);
    expect(overlaps(shortcut("k", "meta"), shortcut("k", "meta", "shift"))).toBe(false);
  });
  it("normalizes malformed data without losing valid unknown entries", () => {
    expect(
      normalizeOverrides({
        "future.action": [shortcut("X", "meta", "meta")],
        bad: [{ key: "Cmd+K+R", modifiers: [] }],
        badModifier: [{ key: "x", modifiers: ["super"] }],
        "list.new": [],
      })
    ).toEqual({ "future.action": [shortcut("x", "meta")], "list.new": [] });
  });
  it("has unique IDs and no editable defaults that conflict", () => {
    expect(new Set(bindings.map((v) => v.id)).size).toBe(bindings.length);
    for (const binding of bindings.filter((v) => v.editable))
      expect(conflicts(binding, binding.defaults, {}).map((v) => v.id)).toEqual([]);
  });
});

it("protects positional bindings from reassignment and ignores former overrides", () => {
  for (const id of ["workspace.1", "palette.workspace.1", "list.jump.1", "palette.jump.1"]) {
    expect(entry(id).editable).toBe(false);
    expect(effectiveBindings(entry(id), { [id]: [] })).toEqual(entry(id).defaults);
    expect(() => updateBinding({}, id, [shortcut("x")])).toThrow("reserved");
  }
  expect(() => updateBinding({}, "list.new", [shortcut("1", "meta")], true)).toThrow("Reserved");
});

it.each(Array.from({ length: 10 }, (_, i) => String(i)))(
  "reserves bare %s for typing and positional navigation",
  (key) => {
    for (const value of [shortcut(key), shortcut(key, "shift")]) {
      expect(() => updateBinding({}, "palette.new", [value], true)).toThrow(
        "reserved for typing and navigation"
      );
      expect(effectiveBindings(entry("palette.new"), { "palette.new": [value] })).toEqual([]);
    }
    expect(() => updateBinding({}, "palette.new", [shortcut(key, "meta")], true)).toThrow(
      "Reserved"
    );
    expect(() => updateBinding({}, "list.new", [shortcut(key, "meta")], true)).toThrow("Reserved");
  }
);

it("allows modified numbers when no positional binding reserves the combination", () => {
  expect(updateBinding({}, "list.new", [shortcut("2", "alt")])).toEqual({
    "list.new": [shortcut("2", "alt")],
  });
});
