import type { ITheme } from "@xterm/xterm";

const cssVar = (name: string) =>
  getComputedStyle(document.documentElement).getPropertyValue(name).trim();

const LIGHT_ANSI_COLORS: ITheme = {
  black: "#1a1a24",
  red: "#c4262e",
  green: "#2f7d3b",
  yellow: "#9a6700",
  blue: "#2563eb",
  magenta: "#a21caf",
  cyan: "#0e7490",
  white: "#6b6b80",
  brightBlack: "#5c5c72",
  brightRed: "#dc2626",
  brightGreen: "#16a34a",
  brightYellow: "#b45309",
  brightBlue: "#3b82f6",
  brightMagenta: "#c026d3",
  brightCyan: "#0891b2",
  brightWhite: "#8a8aa0",
};

/** Terminal colors follow the active app theme (CSS variables set by `applyTheme`). */
export function xtermThemeFromCss(): ITheme {
  const foreground = cssVar("--color-text-primary") || "#e8e8ed";
  const isLight = document.documentElement.dataset.appearance === "light";
  return {
    ...(isLight ? LIGHT_ANSI_COLORS : {}),
    background: cssVar("--color-bg-primary") || "#0a0a0f",
    foreground,
    cursor: foreground,
    selectionBackground: `${cssVar("--color-accent") || "#6366f1"}55`,
  };
}
