import { themeAppearance } from "./themes";

type Rgb = [number, number, number];

const HEX_COLOR = /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;

function parseRgba(value: string): { rgb: Rgb; alpha: number } | null {
  const trimmed = value.trim();
  if (!HEX_COLOR.test(trimmed)) return null;
  const digits = trimmed.slice(1);
  const full = digits.length <= 4 ? [...digits].map((d) => d + d).join("") : digits;
  const channel = (index: number) => parseInt(full.slice(index * 2, index * 2 + 2), 16);
  return {
    rgb: [channel(0), channel(1), channel(2)],
    alpha: full.length === 8 ? channel(3) / 255 : 1,
  };
}

function toHex(rgb: Rgb): string {
  return `#${rgb.map((c) => Math.round(c).toString(16).padStart(2, "0")).join("")}`;
}

/** Mixes `from` toward `to` by `amount` (0 keeps `from`, 1 gives `to`). */
export function mixColors(from: string, to: string, amount: number): string {
  const a = parseRgba(from)?.rgb ?? [0, 0, 0];
  const b = parseRgba(to)?.rgb ?? [0, 0, 0];
  return toHex([0, 1, 2].map((i) => a[i] + (b[i] - a[i]) * amount) as Rgb);
}

/** A `#rrggbb` color, compositing any alpha over `background`; `null` for anything but hex. */
export function opaqueHex(value: unknown, background: string): string | null {
  if (typeof value !== "string") return null;
  const parsed = parseRgba(value);
  if (!parsed) return null;
  return mixColors(background, toHex(parsed.rgb), parsed.alpha);
}

const STATUS_COLORS = {
  dark: { success: "#22c55e", warning: "#f59e0b", danger: "#ef4444", "danger-hover": "#dc2626" },
  light: { success: "#16a34a", warning: "#b45309", danger: "#dc2626", "danger-hover": "#b91c1c" },
};

const STEPS_TOWARD_TEXT = {
  "bg-secondary": 0.03,
  "bg-tertiary": 0.06,
  "bg-hover": 0.09,
  "bg-active": 0.12,
  border: 0.12,
  "border-light": 0.2,
  "text-muted": 0.4,
  "text-secondary": 0.58,
  "text-primary": 0.88,
};

/** Every theme token, derived from a background and an accent color. */
export function generateThemeColors(background: string, accent: string): Record<string, string> {
  const base = opaqueHex(background, "#000000") ?? "#000000";
  const accentColor = opaqueHex(accent, base) ?? base;
  const appearance = themeAppearance({ "bg-primary": base });
  const ink = appearance === "dark" ? "#ffffff" : "#000000";
  const layers = Object.fromEntries(
    Object.entries(STEPS_TOWARD_TEXT).map(([key, amount]) => [key, mixColors(base, ink, amount)])
  );
  return {
    "bg-primary": base,
    ...layers,
    accent: accentColor,
    "accent-hover": mixColors(accentColor, ink, 0.2),
    ...STATUS_COLORS[appearance],
  };
}
