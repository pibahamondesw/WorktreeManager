import { invoke } from "@tauri-apps/api/core";
import { AgentId } from "../types";

type ForkMethod = "inSession" | "relaunch";

/** What an agent supports, as declared by its backend adapter. */
interface AgentCapabilities {
  fork: ForkMethod | null;
  review: boolean;
  rename: boolean;
  archive: boolean;
  models: boolean;
  effort: boolean;
  permissionModes: string[];
  interrupt: boolean;
  /** A message sent during a turn steers it instead of waiting in a queue. */
  steer: boolean;
  compact: boolean;
  images: boolean;
  headless: boolean;
  /** Unattended sessions enforce per-session allow and deny tool rules. */
  toolRules: boolean;
}

export interface AgentDescriptor {
  id: AgentId;
  label: string;
  installed: boolean;
  capabilities: AgentCapabilities;
}

/** What the UI shows about an agent that its descriptor does not carry yet. */
interface AgentPresentation {
  planUrl?: string;
  /** Usage windows shown on the compact indicator; every window when omitted. */
  primaryUsageWindows?: string[];
  alertHooksConfig?: string;
  alertHooksTrust?: string;
  /** Static Tailwind classes in the agent's color, so the active agent is obvious at a glance. */
  tone: { chip: string; dot: string; focus: string };
}

const NEUTRAL_TONE: AgentPresentation["tone"] = {
  chip: "bg-bg-tertiary text-text-secondary hover:bg-bg-hover",
  dot: "bg-text-muted",
  focus: "focus-within:border-accent",
};

const PRESENTATION: Record<AgentId, AgentPresentation> = {
  claude: {
    primaryUsageWindows: ["five_hour", "seven_day"],
    alertHooksConfig: "~/.claude/settings.json",
    tone: {
      chip: "bg-agent-claude/15 text-agent-claude hover:bg-agent-claude/25",
      dot: "bg-agent-claude",
      focus: "focus-within:border-agent-claude",
    },
  },
  codex: {
    planUrl: "https://learn.chatgpt.com/docs/pricing",
    alertHooksConfig: "~/.codex/hooks.json",
    alertHooksTrust:
      "Run /hooks in the Codex chat or CLI to review and trust new or changed hooks.",
    tone: {
      chip: "bg-agent-codex/15 text-agent-codex hover:bg-agent-codex/25",
      dot: "bg-agent-codex",
      focus: "focus-within:border-agent-codex",
    },
  },
};

export const agentPresentation = (id: AgentId): AgentPresentation =>
  Object.hasOwn(PRESENTATION, id) ? PRESENTATION[id] : { tone: NEUTRAL_TONE };

const NO_CAPABILITIES: AgentCapabilities = {
  fork: null,
  review: false,
  rename: false,
  archive: false,
  models: false,
  effort: false,
  permissionModes: [],
  interrupt: false,
  steer: false,
  compact: false,
  images: false,
  headless: false,
  toolRules: false,
};

let lastCatalog: AgentDescriptor[] = [];
let pending: Promise<AgentDescriptor[]> | null = null;

/** Reads the catalog again (installed state can change); concurrent callers share one read. */
export function loadAgentCatalog(): Promise<AgentDescriptor[]> {
  pending ??= invoke<AgentDescriptor[]>("agents_catalog")
    .then((catalog) => {
      if (!Array.isArray(catalog)) throw new Error("Could not read the agent catalog");
      lastCatalog = catalog;
      return catalog;
    })
    .finally(() => {
      pending = null;
    });
  return pending;
}

/** The last catalog read, empty until the first read finishes. */
export const cachedAgentCatalog = () => lastCatalog;

/** An id outside the catalog (a persisted agent that is gone) keeps its id as label. */
export const describeAgent = (
  id: AgentId,
  catalog: AgentDescriptor[] = lastCatalog
): AgentDescriptor =>
  catalog.find((agent) => agent.id === id) ?? {
    id,
    label: id,
    installed: false,
    capabilities: NO_CAPABILITIES,
  };

export async function isCatalogAgent(id: string): Promise<boolean> {
  const catalog = lastCatalog.length ? lastCatalog : await loadAgentCatalog();
  return catalog.some((agent) => agent.id === id);
}

/**
 * The installed agents that can run a chat, in catalog order; `current` always counts so the open
 * chat stays listed.
 */
export function chatAgents(catalog: AgentDescriptor[], current: AgentId): AgentId[] {
  const ids = catalog
    .filter((agent) => agent.id === current || (agent.installed && agent.capabilities.headless))
    .map((agent) => agent.id);
  return ids.includes(current) ? ids : [...ids, current];
}
