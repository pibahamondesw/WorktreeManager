export interface Theme {
  id: string;
  name: string;
  colors: Record<string, string>;
}

export type ThemeAppearance = "light" | "dark";

/** Id of the user-configurable theme (not part of the `themes` preset list). */
export const CUSTOM_THEME_ID = "custom";

/** Id of the theme that follows the macOS appearance (not part of the `themes` preset list). */
export const SYSTEM_THEME_ID = "system";

const SYSTEM_DARK_THEME_ID = "default";
const SYSTEM_LIGHT_THEME_ID = "light";

export const THEME_CHANGE_EVENT = "app-theme-change";

/** A single editable color token, used to render the custom theme editor. */
export interface ThemeColorToken {
  key: string;
  label: string;
  group: string;
}

/**
 * All 16 color tokens in display order, grouped by category. Keys must match
 * the `colors` records of every theme (and the `@theme` block in index.css).
 */
export const THEME_COLOR_TOKENS: ThemeColorToken[] = [
  { key: "bg-primary", label: "Primary background", group: "Background" },
  { key: "bg-secondary", label: "Secondary background", group: "Background" },
  { key: "bg-tertiary", label: "Tertiary background", group: "Background" },
  { key: "bg-hover", label: "Hover background", group: "Background" },
  { key: "bg-active", label: "Active background", group: "Background" },
  { key: "border", label: "Border", group: "Border" },
  { key: "border-light", label: "Light border", group: "Border" },
  { key: "text-primary", label: "Primary text", group: "Text" },
  { key: "text-secondary", label: "Secondary text", group: "Text" },
  { key: "text-muted", label: "Muted text", group: "Text" },
  { key: "accent", label: "Accent", group: "Accent" },
  { key: "accent-hover", label: "Accent hover", group: "Accent" },
  { key: "success", label: "Success", group: "Status" },
  { key: "warning", label: "Warning", group: "Status" },
  { key: "danger", label: "Danger", group: "Status" },
  { key: "danger-hover", label: "Danger hover", group: "Status" },
];

export const themes: Theme[] = [
  {
    id: "default",
    name: "Indigo",
    colors: {
      "bg-primary": "#0a0a0f",
      "bg-secondary": "#12121a",
      "bg-tertiary": "#1a1a25",
      "bg-hover": "#22222f",
      "bg-active": "#2a2a38",
      border: "#2a2a38",
      "border-light": "#3a3a48",
      "text-primary": "#e8e8ed",
      "text-secondary": "#8888a0",
      "text-muted": "#5a5a70",
      accent: "#6366f1",
      "accent-hover": "#818cf8",
      success: "#22c55e",
      warning: "#f59e0b",
      danger: "#ef4444",
      "danger-hover": "#dc2626",
    },
  },
  {
    id: "ocean",
    name: "Ocean",
    colors: {
      "bg-primary": "#0b1120",
      "bg-secondary": "#0f1729",
      "bg-tertiary": "#162032",
      "bg-hover": "#1c2a40",
      "bg-active": "#24344c",
      border: "#1e3050",
      "border-light": "#2a4060",
      "text-primary": "#e2e8f0",
      "text-secondary": "#7b92b0",
      "text-muted": "#4a6080",
      accent: "#38bdf8",
      "accent-hover": "#7dd3fc",
      success: "#34d399",
      warning: "#fbbf24",
      danger: "#f87171",
      "danger-hover": "#ef4444",
    },
  },
  {
    id: "forest",
    name: "Forest",
    colors: {
      "bg-primary": "#0a100e",
      "bg-secondary": "#101a16",
      "bg-tertiary": "#18241e",
      "bg-hover": "#1f2e26",
      "bg-active": "#283830",
      border: "#283830",
      "border-light": "#384840",
      "text-primary": "#e4ede8",
      "text-secondary": "#7ea090",
      "text-muted": "#4a7060",
      accent: "#10b981",
      "accent-hover": "#34d399",
      success: "#22c55e",
      warning: "#fbbf24",
      danger: "#f87171",
      "danger-hover": "#ef4444",
    },
  },
  {
    id: "sunset",
    name: "Sunset",
    colors: {
      "bg-primary": "#120a08",
      "bg-secondary": "#1a1210",
      "bg-tertiary": "#241a16",
      "bg-hover": "#2e221e",
      "bg-active": "#382c28",
      border: "#382c28",
      "border-light": "#483c38",
      "text-primary": "#f0e8e4",
      "text-secondary": "#a08880",
      "text-muted": "#705a50",
      accent: "#f97316",
      "accent-hover": "#fb923c",
      success: "#22c55e",
      warning: "#fbbf24",
      danger: "#ef4444",
      "danger-hover": "#dc2626",
    },
  },
  {
    id: "rose",
    name: "Rose",
    colors: {
      "bg-primary": "#100a0e",
      "bg-secondary": "#181218",
      "bg-tertiary": "#221a22",
      "bg-hover": "#2c222c",
      "bg-active": "#362c36",
      border: "#362c36",
      "border-light": "#463c46",
      "text-primary": "#f0e8ee",
      "text-secondary": "#a088a0",
      "text-muted": "#705a70",
      accent: "#f43f5e",
      "accent-hover": "#fb7185",
      success: "#22c55e",
      warning: "#fbbf24",
      danger: "#ef4444",
      "danger-hover": "#dc2626",
    },
  },
  {
    id: "nord",
    name: "Nord",
    colors: {
      "bg-primary": "#2e3440",
      "bg-secondary": "#3b4252",
      "bg-tertiary": "#434c5e",
      "bg-hover": "#4c566a",
      "bg-active": "#555e70",
      border: "#4c566a",
      "border-light": "#5e6778",
      "text-primary": "#eceff4",
      "text-secondary": "#a0aabb",
      "text-muted": "#6e7a8a",
      accent: "#88c0d0",
      "accent-hover": "#8fbcbb",
      success: "#a3be8c",
      warning: "#ebcb8b",
      danger: "#bf616a",
      "danger-hover": "#a5444d",
    },
  },
  {
    id: "dracula",
    name: "Dracula",
    colors: {
      "bg-primary": "#21222c",
      "bg-secondary": "#282a36",
      "bg-tertiary": "#2e303e",
      "bg-hover": "#363848",
      "bg-active": "#3e4052",
      border: "#44475a",
      "border-light": "#565970",
      "text-primary": "#f8f8f2",
      "text-secondary": "#9898b0",
      "text-muted": "#6272a4",
      accent: "#bd93f9",
      "accent-hover": "#d0a8ff",
      success: "#50fa7b",
      warning: "#f1fa8c",
      danger: "#ff5555",
      "danger-hover": "#e03c3c",
    },
  },
  {
    id: "mono",
    name: "Monochrome",
    colors: {
      "bg-primary": "#0c0c0c",
      "bg-secondary": "#141414",
      "bg-tertiary": "#1c1c1c",
      "bg-hover": "#242424",
      "bg-active": "#2c2c2c",
      border: "#2c2c2c",
      "border-light": "#3c3c3c",
      "text-primary": "#e8e8e8",
      "text-secondary": "#888888",
      "text-muted": "#585858",
      accent: "#a0a0a0",
      "accent-hover": "#c0c0c0",
      success: "#6fcf6f",
      warning: "#cfcf4f",
      danger: "#cf4f4f",
      "danger-hover": "#b03030",
    },
  },
  {
    id: "light",
    name: "Daylight",
    colors: {
      "bg-primary": "#fbfbfd",
      "bg-secondary": "#f3f3f8",
      "bg-tertiary": "#eaeaf1",
      "bg-hover": "#e2e2eb",
      "bg-active": "#d8d8e4",
      border: "#dcdce6",
      "border-light": "#c6c6d4",
      "text-primary": "#1a1a24",
      "text-secondary": "#5c5c72",
      "text-muted": "#8a8aa0",
      accent: "#4f46e5",
      "accent-hover": "#4338ca",
      success: "#16a34a",
      warning: "#b45309",
      danger: "#dc2626",
      "danger-hover": "#b91c1c",
    },
  },
  {
    id: "catppuccin-latte",
    name: "Catppuccin Latte",
    colors: {
      "bg-primary": "#eff1f5",
      "bg-secondary": "#e6e9ef",
      "bg-tertiary": "#dce0e8",
      "bg-hover": "#d5d9e2",
      "bg-active": "#ccd0da",
      border: "#ccd0da",
      "border-light": "#bcc0cc",
      "text-primary": "#4c4f69",
      "text-secondary": "#6c6f85",
      "text-muted": "#8c8fa1",
      accent: "#8839ef",
      "accent-hover": "#7024d6",
      success: "#40a02b",
      warning: "#c27612",
      danger: "#d20f39",
      "danger-hover": "#b00c2f",
    },
  },
  {
    id: "rose-pine-dawn",
    name: "Rosé Pine Dawn",
    colors: {
      "bg-primary": "#faf4ed",
      "bg-secondary": "#f4ede8",
      "bg-tertiary": "#f2e9e1",
      "bg-hover": "#ebe2da",
      "bg-active": "#dfdad9",
      border: "#dfdad9",
      "border-light": "#cecacd",
      "text-primary": "#575279",
      "text-secondary": "#797593",
      "text-muted": "#9893a5",
      accent: "#907aa9",
      "accent-hover": "#7c6595",
      success: "#286983",
      warning: "#c9801f",
      danger: "#b4637a",
      "danger-hover": "#9d5168",
    },
  },
  {
    id: "gruvbox-light",
    name: "Gruvbox Light",
    colors: {
      "bg-primary": "#fbf1c7",
      "bg-secondary": "#f2e5bc",
      "bg-tertiary": "#ebdbb2",
      "bg-hover": "#e3d2a8",
      "bg-active": "#d5c4a1",
      border: "#d5c4a1",
      "border-light": "#bdae93",
      "text-primary": "#3c3836",
      "text-secondary": "#665c54",
      "text-muted": "#928374",
      accent: "#af3a03",
      "accent-hover": "#8f3202",
      success: "#79740e",
      warning: "#b57614",
      danger: "#cc241d",
      "danger-hover": "#9d0006",
    },
  },
  {
    id: "everforest-light",
    name: "Everforest Light",
    colors: {
      "bg-primary": "#fffbef",
      "bg-secondary": "#f8f5e4",
      "bg-tertiary": "#f2efdf",
      "bg-hover": "#edeada",
      "bg-active": "#e8e5d5",
      border: "#e8e5d5",
      "border-light": "#bec5b2",
      "text-primary": "#5c6a72",
      "text-secondary": "#829181",
      "text-muted": "#a6b0a0",
      accent: "#35a77c",
      "accent-hover": "#2b8a66",
      success: "#7f9100",
      warning: "#c08a00",
      danger: "#f85552",
      "danger-hover": "#e5403d",
    },
  },
  {
    id: "solarized-light",
    name: "Solarized Light",
    colors: {
      "bg-primary": "#fdf6e3",
      "bg-secondary": "#f5efdc",
      "bg-tertiary": "#eee8d5",
      "bg-hover": "#e6dfca",
      "bg-active": "#ddd6c1",
      border: "#e0d9c4",
      "border-light": "#cdc5ad",
      "text-primary": "#073642",
      "text-secondary": "#586e75",
      "text-muted": "#93a1a1",
      accent: "#268bd2",
      "accent-hover": "#1f72ad",
      success: "#859900",
      warning: "#b58900",
      danger: "#dc322f",
      "danger-hover": "#b8281f",
    },
  },
];

function relativeLuminance(hex: string) {
  const channels = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  const [r, g, b] = channels.map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function themeAppearance(colors: Record<string, string>): ThemeAppearance {
  return relativeLuminance(colors["bg-primary"] ?? "#000000") > 0.5 ? "light" : "dark";
}

function systemPrefersDark() {
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true;
}

const presetById = (id: string) => themes.find((t) => t.id === id) ?? themes[0];

export const systemThemePresets = {
  light: presetById(SYSTEM_LIGHT_THEME_ID),
  dark: presetById(SYSTEM_DARK_THEME_ID),
};

/** The preset a theme id renders with; `system` resolves through the macOS appearance. */
export function presetFor(themeId: string): Theme {
  if (themeId !== SYSTEM_THEME_ID) return presetById(themeId);
  return systemPrefersDark() ? systemThemePresets.dark : systemThemePresets.light;
}

export function applyTheme(themeId: string, customColors?: Record<string, string>) {
  const colors =
    themeId === CUSTOM_THEME_ID ? (customColors ?? themes[0].colors) : presetFor(themeId).colors;
  const root = document.documentElement;
  for (const [key, value] of Object.entries(colors)) {
    root.style.setProperty(`--color-${key}`, value);
  }
  const appearance = themeAppearance(colors);
  root.dataset.appearance = appearance;
  root.style.colorScheme = appearance;
  window.dispatchEvent(new Event(THEME_CHANGE_EVENT));
}
