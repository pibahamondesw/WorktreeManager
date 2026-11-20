// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import {
  adjustThemeContrast,
  applyTypography,
  defaultTypography,
  fontStack,
  setContrast,
  terminalFont,
} from "./appearance";
import { applyTheme, THEME_CHANGE_EVENT, themes } from "./themes";

const root = document.documentElement;
const cssVar = (name: string) => root.style.getPropertyValue(name);

afterEach(() => {
  root.removeAttribute("style");
  setContrast(100);
  window.removeEventListener(THEME_CHANGE_EVENT, adjustThemeContrast);
});

describe("fontStack", () => {
  it("quotes multi-word families, strips CSS syntax and keeps the fallback", () => {
    expect(fontStack(' Fira Code , "SF Mono", Menlo;}', "monospace")).toBe(
      '"Fira Code", "SF Mono", Menlo, monospace'
    );
    expect(fontStack(" , ", "monospace")).toBe("monospace");
  });
});

describe("applyTypography", () => {
  it("sets each area's variables, chaining defaults from interface to chat and code to terminal", () => {
    const typography = defaultTypography();
    applyTypography({
      ...typography,
      ui: { family: "Inter", size: 15 },
      code: { family: "Fira Code", size: 13 },
    });

    expect(cssVar("--font-ui")).toContain("Inter, -apple-system");
    expect(cssVar("--font-size-ui")).toBe("15px");
    expect(cssVar("--font-chat")).toBe(cssVar("--font-ui"));
    expect(cssVar("--font-size-chat")).toBe("calc(14px * var(--ui-zoom))");
    expect(cssVar("--font-code")).toBe('"Fira Code", Menlo, Monaco, monospace');
    expect(cssVar("--font-size-code")).toBe("calc(13px * var(--ui-zoom))");
    expect(terminalFont()).toEqual({
      fontFamily: '"Fira Code", Menlo, Monaco, monospace',
      fontSize: 12,
    });
  });

  it("falls back to the default terminal font before typography is applied", () => {
    expect(terminalFont()).toEqual({ fontFamily: "Menlo, Monaco, monospace", fontSize: 12 });
  });
});

describe("adjustThemeContrast", () => {
  const theme = themes[0].colors;

  it("keeps the theme colors at the default contrast", () => {
    window.addEventListener(THEME_CHANGE_EVENT, adjustThemeContrast);
    applyTheme(themes[0].id);
    expect(cssVar("--color-text-muted")).toBe(theme["text-muted"]);
  });

  it("mixes secondary text toward the primary text and borders half as much when raised", () => {
    window.addEventListener(THEME_CHANGE_EVENT, adjustThemeContrast);
    setContrast(140);
    applyTheme(themes[0].id);
    expect(cssVar("--color-text-muted")).toBe(
      `color-mix(in srgb, ${theme["text-muted"]} 60%, ${theme["text-primary"]})`
    );
    expect(cssVar("--color-border")).toBe(
      `color-mix(in srgb, ${theme.border} 80%, ${theme["text-primary"]})`
    );
  });

  it("mixes toward the background when lowered and recomputes from each theme", () => {
    window.addEventListener(THEME_CHANGE_EVENT, adjustThemeContrast);
    setContrast(80);
    applyTheme(themes[0].id);
    applyTheme(themes[0].id);
    expect(cssVar("--color-text-secondary")).toBe(
      `color-mix(in srgb, ${theme["text-secondary"]} 80%, ${theme["bg-primary"]})`
    );
  });

  it("skips colors the theme did not set", () => {
    setContrast(120);
    adjustThemeContrast();
    expect(cssVar("--color-text-muted")).toBe("");
  });
});
