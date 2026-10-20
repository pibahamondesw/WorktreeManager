// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  applyTheme,
  presetFor,
  themeAppearance,
  themes,
  CUSTOM_THEME_ID,
  SYSTEM_THEME_ID,
  THEME_CHANGE_EVENT,
  THEME_COLOR_TOKENS,
} from "./themes";

function mockSystemDark(dark: boolean) {
  vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({ matches: dark } as MediaQueryList));
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("themes", () => {
  it("defines every color token for every preset and offers both appearances", () => {
    const keys = THEME_COLOR_TOKENS.map((t) => t.key).sort();
    for (const theme of themes) expect(Object.keys(theme.colors).sort()).toEqual(keys);
    const appearances = new Set(themes.map((t) => themeAppearance(t.colors)));
    expect(appearances).toEqual(new Set(["light", "dark"]));
  });

  it("classifies appearance from the primary background", () => {
    expect(themeAppearance({ "bg-primary": "#fdf6e3" })).toBe("light");
    expect(themeAppearance({ "bg-primary": "#2e3440" })).toBe("dark");
  });

  it("resolves the system theme from the macOS appearance", () => {
    mockSystemDark(true);
    expect(themeAppearance(presetFor(SYSTEM_THEME_ID).colors)).toBe("dark");
    mockSystemDark(false);
    expect(themeAppearance(presetFor(SYSTEM_THEME_ID).colors)).toBe("light");
  });

  it("falls back to the first preset for unknown ids", () => {
    expect(presetFor("removed-theme")).toBe(themes[0]);
  });

  it("applies colors, appearance and notifies listeners", () => {
    const listener = vi.fn();
    window.addEventListener(THEME_CHANGE_EVENT, listener);
    const root = document.documentElement;

    applyTheme("solarized-light");
    expect(root.style.getPropertyValue("--color-bg-primary")).toBe("#fdf6e3");
    expect(root.dataset.appearance).toBe("light");
    expect(root.style.colorScheme).toBe("light");

    applyTheme(CUSTOM_THEME_ID, { ...themes[0].colors });
    expect(root.dataset.appearance).toBe("dark");
    expect(listener).toHaveBeenCalledTimes(2);
    window.removeEventListener(THEME_CHANGE_EVENT, listener);
  });
});
