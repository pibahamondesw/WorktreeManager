import { AgentId, TaskSurface } from "../types";
import { AgentActivity, embeddedSurface } from "../services/agentActivity";
import { isEmbedded } from "./taskSurface";

export type TaskOpenPlan =
  | { kind: "open"; surface: TaskSurface }
  | { kind: "confirm"; current: TaskSurface; requested: TaskSurface; closing: TaskSurface[] };

export type SurfaceSwitchChoice = "keep" | "replace" | "cancel";

const AGENT_PRODUCTS: Record<AgentId | "opencode", string> = {
  claude: "Claude Code",
  codex: "Codex",
  opencode: "OpenCode",
};

const agentOf = (surface: TaskSurface) => ("agent" in surface ? surface.agent : null);

const sameSurface = (a: TaskSurface, b: TaskSurface) =>
  a.kind === b.kind && agentOf(a) === agentOf(b);

/**
 * A task keeps one live embedded surface. Opening a different embedded surface over a live one
 * needs the user to choose; external editors close nothing and open directly.
 */
export function planTaskOpen(live: TaskSurface[], requested: TaskSurface): TaskOpenPlan {
  const [current] = live;
  if (!current || !isEmbedded(requested) || live.some((surface) => sameSurface(surface, requested)))
    return { kind: "open", surface: requested };
  return { kind: "confirm", current, requested, closing: live };
}

/** Where a navigation that must not ask or close anything lands: the live surface if any. */
export const landingSurface = (live: TaskSurface[], requested: TaskSurface): TaskSurface =>
  live.find((surface) => sameSurface(surface, requested)) ?? live[0] ?? requested;

export function surfaceLabel(surface: TaskSurface): string {
  switch (surface.kind) {
    case "editor":
      return "VSCode";
    case "opencode":
      return "OpenCode";
    case "chat":
      return `${AGENT_PRODUCTS[surface.agent]} chat`;
    case "terminal":
      return `${AGENT_PRODUCTS[surface.agent]} terminal`;
    case "external":
      return "an external editor";
  }
}

/** The agent whose turn closing these surfaces would interrupt, if one is working in them. */
export function interruptedAgent(closing: TaskSurface[], activity?: AgentActivity): string | null {
  if (activity?.state !== "working") return null;
  const working = embeddedSurface(activity);
  if (!working || !closing.some((surface) => sameSurface(surface, working))) return null;
  return AGENT_PRODUCTS[activity.agent];
}
