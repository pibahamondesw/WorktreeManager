export const FONT_AREAS = ["ui", "chat", "code", "terminal"] as const;
export type FontArea = (typeof FONT_AREAS)[number];

interface FontSetting {
  /** Comma-separated font families; empty uses the area's default. */
  family: string;
  size: number;
}

export type Typography = Record<FontArea, FontSetting>;

export const FONT_SIZE_LIMITS: Record<FontArea, { min: number; max: number; default: number }> = {
  ui: { min: 12, max: 20, default: 16 },
  chat: { min: 11, max: 20, default: 14 },
  code: { min: 10, max: 20, default: 12 },
  terminal: { min: 9, max: 24, default: 12 },
};

const DEFAULT_UI_FONT_STACK =
  '-apple-system, BlinkMacSystemFont, "Segoe UI", "Helvetica Neue", sans-serif';
const DEFAULT_CODE_FONT_STACK = "Menlo, Monaco, monospace";

export const CONTRAST_LIMITS = { min: 50, max: 150, default: 100 };

export const TYPOGRAPHY_CHANGE_EVENT = "app-typography-change";

function clampSize(area: FontArea, value: unknown): number {
  const { min, max, default: fallback } = FONT_SIZE_LIMITS[area];
  return typeof value === "number" && Number.isFinite(value)
    ? Math.min(max, Math.max(min, Math.round(value)))
    : fallback;
}

export function defaultTypography(): Typography {
  return normalizeTypography(undefined);
}

export function normalizeTypography(raw: unknown): Typography {
  const value = raw !== null && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const setting = (area: FontArea): FontSetting => {
    const entry =
      value[area] !== null && typeof value[area] === "object"
        ? (value[area] as Record<string, unknown>)
        : {};
    return {
      family: typeof entry.family === "string" ? entry.family.trim() : "",
      size: clampSize(area, entry.size),
    };
  };
  return {
    ui: setting("ui"),
    chat: setting("chat"),
    code: setting("code"),
    terminal: setting("terminal"),
  };
}

export function normalizeContrast(raw: unknown): number {
  const { min, max, default: fallback } = CONTRAST_LIMITS;
  return typeof raw === "number" && raw >= min && raw <= max ? Math.round(raw) : fallback;
}

function quoteFamily(name: string): string {
  const bare = name.replace(/["'\\;{}<>]/g, "").trim();
  if (!bare) return "";
  return /^[a-zA-Z][a-zA-Z0-9-]*$/.test(bare) ? bare : `"${bare}"`;
}

/** The CSS font stack for a custom family list, falling back to `fallback`. */
export function fontStack(custom: string, fallback: string): string {
  const families = custom.split(",").map(quoteFamily).filter(Boolean);
  return families.length ? `${families.join(", ")}, ${fallback}` : fallback;
}

const zoomed = (px: number) => `calc(${px}px * var(--ui-zoom))`;

export function applyTypography(typography: Typography) {
  const root = document.documentElement;
  const uiStack = fontStack(typography.ui.family, DEFAULT_UI_FONT_STACK);
  const codeStack = fontStack(typography.code.family, DEFAULT_CODE_FONT_STACK);
  root.style.setProperty("--font-ui", uiStack);
  root.style.setProperty("--font-size-ui", `${typography.ui.size}px`);
  root.style.setProperty("--font-chat", fontStack(typography.chat.family, uiStack));
  root.style.setProperty("--font-size-chat", zoomed(typography.chat.size));
  root.style.setProperty("--font-code", codeStack);
  root.style.setProperty("--font-size-code", zoomed(typography.code.size));
  root.style.setProperty("--font-terminal", fontStack(typography.terminal.family, codeStack));
  root.style.setProperty("--font-size-terminal", String(typography.terminal.size));
  window.dispatchEvent(new Event(TYPOGRAPHY_CHANGE_EVENT));
}

/** The terminal font, as set by `applyTypography`. */
export function terminalFont(): { fontFamily: string; fontSize: number } {
  const style = getComputedStyle(document.documentElement);
  const size = Number.parseFloat(style.getPropertyValue("--font-size-terminal"));
  return {
    fontFamily: style.getPropertyValue("--font-terminal").trim() || DEFAULT_CODE_FONT_STACK,
    fontSize: Number.isFinite(size) ? size : FONT_SIZE_LIMITS.terminal.default,
  };
}

const CONTRAST_TARGETS = [
  { variable: "text-secondary", weight: 1 },
  { variable: "text-muted", weight: 1 },
  { variable: "border", weight: 0.5 },
  { variable: "border-light", weight: 0.5 },
];

let contrastLevel = CONTRAST_LIMITS.default;

export function setContrast(contrast: number) {
  contrastLevel = contrast;
}

/**
 * Moves secondary text and borders toward the primary text color (more contrast) or the
 * background (less). Runs after each `applyTheme`, from the base colors it just set.
 */
export function adjustThemeContrast() {
  const shift = contrastLevel - CONTRAST_LIMITS.default;
  if (shift === 0) return;
  const root = document.documentElement;
  const themeColor = (name: string) => root.style.getPropertyValue(`--color-${name}`).trim();
  const towards = themeColor(shift > 0 ? "text-primary" : "bg-primary");
  for (const { variable, weight } of CONTRAST_TARGETS) {
    const base = themeColor(variable);
    if (!base || !towards) continue;
    root.style.setProperty(
      `--color-${variable}`,
      `color-mix(in srgb, ${base} ${100 - Math.abs(shift) * weight}%, ${towards})`
    );
  }
}
