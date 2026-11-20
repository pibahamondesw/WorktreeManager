import { describe, expect, it } from "vitest";
import { generateThemeColors, mixColors, opaqueHex } from "./themePalette";
import { themeAppearance, THEME_COLOR_TOKENS } from "./themes";

describe("generateThemeColors", () => {
  it("fills every token from a dark background, layering toward white", () => {
    const colors = generateThemeColors("#101820", "#3b82f6");
    expect(Object.keys(colors).sort()).toEqual(THEME_COLOR_TOKENS.map((t) => t.key).sort());
    expect(colors["bg-primary"]).toBe("#101820");
    expect(colors.accent).toBe("#3b82f6");
    expect(themeAppearance(colors)).toBe("dark");
    expect(colors["bg-secondary"]).toBe(mixColors("#101820", "#ffffff", 0.03));
    expect(colors["accent-hover"]).toBe(mixColors("#3b82f6", "#ffffff", 0.2));
    expect(colors.danger).toBe("#ef4444");
  });

  it("layers a light background toward black with darker status colors", () => {
    const colors = generateThemeColors("#fafafa", "#f0a");
    expect(themeAppearance(colors)).toBe("light");
    expect(colors.accent).toBe("#ff00aa");
    expect(colors["text-primary"]).toBe(mixColors("#fafafa", "#000000", 0.88));
    expect(colors.danger).toBe("#dc2626");
  });

  it("falls back to black for colors that are not hex", () => {
    const colors = generateThemeColors("red", "nope");
    expect(colors["bg-primary"]).toBe("#000000");
    expect(colors.accent).toBe("#000000");
  });
});

describe("opaqueHex", () => {
  it("composites alpha over the background and rejects non-hex values", () => {
    expect(opaqueHex("#ffffff80", "#000000")).toBe("#808080");
    expect(opaqueHex("#fff8", "#000000")).toBe("#888888");
    expect(opaqueHex(" #ABCDEF ", "#000000")).toBe("#abcdef");
    expect(opaqueHex("rgb(0,0,0)", "#000000")).toBeNull();
    expect(opaqueHex(42, "#000000")).toBeNull();
  });

  it("treats an unparseable mix endpoint as black", () => {
    expect(mixColors("bad", "#ffffff", 0.5)).toBe("#808080");
    expect(mixColors("#ffffff", "bad", 1)).toBe("#000000");
  });
});
