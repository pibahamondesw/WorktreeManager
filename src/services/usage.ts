export interface UsageWindow {
  id: string;
  label: string;
  usedPercent: number;
  /** Unix seconds. */
  resetsAt: number | null;
}

export interface PlanUsage {
  supported: boolean;
  plan: string | null;
  account: string | null;
  windows: UsageWindow[];
  session: string | null;
  error: string | null;
}

export type UsageTone = "normal" | "warning" | "danger";

const USAGE_WARNING_PERCENT = 80;

export function usageTone(usedPercent: number): UsageTone {
  if (usedPercent >= 95) return "danger";
  if (usedPercent >= USAGE_WARNING_PERCENT) return "warning";
  return "normal";
}

export function formatReset(resetsAt: number, now = Date.now()): string {
  const date = new Date(resetsAt * 1000);
  const time = date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  const sameDay = new Date(now).toDateString() === date.toDateString();
  if (sameDay) return `Resets at ${time}`;
  const day = date.toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
  return `Resets ${day}, ${time}`;
}

export const CODEX_PLAN_URL = "https://learn.chatgpt.com/docs/pricing";

export function formatPlanName(plan: string): string {
  const names: Record<string, string> = { prolite: "ProLite", promax: "ProMax" };
  return plan
    .trim()
    .split(/[_\s-]+/)
    .map((word) => names[word.toLowerCase()] ?? word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}
