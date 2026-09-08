import type { ITheme } from "@xterm/xterm";

const cssVar = (name: string) =>
  getComputedStyle(document.documentElement).getPropertyValue(name).trim();

/** Terminal colors follow the active app theme (CSS variables set by `applyTheme`). */
export function xtermThemeFromCss(): ITheme {
  const foreground = cssVar("--color-text-primary") || "#e8e8ed";
  return {
    background: cssVar("--color-bg-primary") || "#0a0a0f",
    foreground,
    cursor: foreground,
    selectionBackground: `${cssVar("--color-accent") || "#6366f1"}55`,
  };
}
