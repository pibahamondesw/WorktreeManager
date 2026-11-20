import { generateThemeColors, mixColors, opaqueHex } from "./themePalette";

export interface VscodeThemeDocument {
  name?: unknown;
  type?: unknown;
  include?: unknown;
  colors?: unknown;
}

export interface ImportedTheme {
  name: string;
  colors: Record<string, string>;
}

/** Parses JSON with comments and trailing commas, as VS Code theme files allow. */
function parseJsonc(source: string): unknown {
  let output = "";
  let index = 0;
  while (index < source.length) {
    const char = source[index];
    if (char === '"') {
      const end = endOfString(source, index);
      output += source.slice(index, end);
      index = end;
    } else if (source.startsWith("//", index)) {
      const newline = source.indexOf("\n", index);
      index = newline === -1 ? source.length : newline;
    } else if (source.startsWith("/*", index)) {
      const close = source.indexOf("*/", index + 2);
      index = close === -1 ? source.length : close + 2;
    } else {
      if (char === "}" || char === "]") output = output.replace(/,\s*$/, "");
      output += char;
      index += 1;
    }
  }
  return JSON.parse(output);
}

function endOfString(source: string, start: number): number {
  let index = start + 1;
  while (index < source.length && source[index] !== '"') {
    index += source[index] === "\\" ? 2 : 1;
  }
  return index + 1;
}

/** A theme file's JSON object; `fileName` names it in errors. */
export function parseThemeDocument(source: string, fileName: string): VscodeThemeDocument {
  let value: unknown;
  try {
    value = parseJsonc(source);
  } catch {
    throw new Error(`${fileName} is not valid JSON.`);
  }
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${fileName} is not a VS Code color theme.`);
  }
  return value as VscodeThemeDocument;
}

/** The theme in a single VS Code theme file. */
export function themeFromFile(source: string, fileName: string): ImportedTheme {
  const theme = parseThemeDocument(source, fileName);
  return {
    name: themeName(theme, fileName.replace(/\.json$/i, "")),
    colors: colorsFromVscodeTheme(theme),
  };
}

const DEFAULT_BACKGROUND = { light: "#ffffff", dark: "#1e1e1e" };
const DEFAULT_ACCENT = "#007acc";

const ACCENT_KEYS = ["button.background", "focusBorder", "textLink.foreground"];

const TOKEN_SOURCES: Record<string, string[]> = {
  "bg-secondary": ["sideBar.background", "activityBar.background"],
  "bg-tertiary": ["input.background", "dropdown.background", "editorWidget.background"],
  "bg-hover": ["list.hoverBackground"],
  "bg-active": ["list.inactiveSelectionBackground", "list.activeSelectionBackground"],
  border: ["panel.border", "sideBar.border", "editorGroup.border"],
  "border-light": ["input.border", "widget.border"],
  "text-primary": ["editor.foreground", "foreground"],
  "text-secondary": ["descriptionForeground", "sideBar.foreground"],
  "text-muted": ["editorLineNumber.foreground", "disabledForeground"],
  "accent-hover": ["button.hoverBackground"],
  success: ["gitDecoration.addedResourceForeground", "terminal.ansiGreen"],
  warning: ["editorWarning.foreground", "terminal.ansiYellow"],
  danger: ["errorForeground", "editorError.foreground", "terminal.ansiRed"],
};

function isLightType(type: unknown): boolean {
  return type === "light" || type === "vs" || type === "hc-light";
}

/** App theme tokens for a VS Code theme, generating whatever the theme leaves unset. */
export function colorsFromVscodeTheme(theme: VscodeThemeDocument): Record<string, string> {
  const source =
    theme.colors !== null && typeof theme.colors === "object"
      ? (theme.colors as Record<string, unknown>)
      : {};
  const fallbackBackground = DEFAULT_BACKGROUND[isLightType(theme.type) ? "light" : "dark"];
  const background =
    opaqueHex(source["editor.background"], fallbackBackground) ?? fallbackBackground;
  const pick = (keys: string[]) =>
    keys.map((key) => opaqueHex(source[key], background)).find((color) => color !== null);
  const colors = generateThemeColors(background, pick(ACCENT_KEYS) ?? DEFAULT_ACCENT);
  for (const [token, keys] of Object.entries(TOKEN_SOURCES)) {
    const color = pick(keys);
    if (color) colors[token] = color;
  }
  if (pick(TOKEN_SOURCES.danger))
    colors["danger-hover"] = mixColors(colors.danger, "#000000", 0.15);
  return colors;
}

export function themeName(theme: VscodeThemeDocument, fallback: string): string {
  return typeof theme.name === "string" && theme.name.trim() ? theme.name.trim() : fallback;
}
