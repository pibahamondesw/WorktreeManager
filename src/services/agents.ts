import { invoke } from "@tauri-apps/api/core";
import { AgentId, EditorApp } from "../types";

/** What the UI shows about each CLI agent; behaviour lives in the backend agent adapters. */
export interface AgentCapabilities {
  label: string;
  productName: string;
  planUrl?: string;
  /** Usage windows shown on the compact indicator; every window when omitted. */
  primaryUsageWindows?: string[];
  alertHooksConfig: string;
  alertHooksTrust?: string;
  steersMidTurn?: boolean;
  /** Static Tailwind classes in the agent's color, so the active agent is obvious at a glance. */
  tone: { chip: string; dot: string; focus: string };
}

export const AGENTS: Record<AgentId, AgentCapabilities> = {
  claude: {
    label: "Claude",
    productName: "Claude Code",
    primaryUsageWindows: ["five_hour", "seven_day"],
    alertHooksConfig: "~/.claude/settings.json",
    tone: {
      chip: "bg-agent-claude/15 text-agent-claude hover:bg-agent-claude/25",
      dot: "bg-agent-claude",
      focus: "focus-within:border-agent-claude",
    },
  },
  codex: {
    label: "Codex",
    productName: "Codex",
    planUrl: "https://learn.chatgpt.com/docs/pricing",
    alertHooksConfig: "~/.codex/hooks.json",
    alertHooksTrust:
      "Run /hooks in the Codex chat or CLI to review and trust new or changed hooks.",
    steersMidTurn: true,
    tone: {
      chip: "bg-agent-codex/15 text-agent-codex hover:bg-agent-codex/25",
      dot: "bg-agent-codex",
      focus: "focus-within:border-agent-codex",
    },
  },
};

export const AGENT_IDS = Object.keys(AGENTS) as AgentId[];

const AGENT_EDITORS: Record<AgentId, EditorApp> = { claude: "claude-code", codex: "codex" };

/** The agents whose CLI is installed; `current` always counts so the open chat stays listed. */
export async function installedAgents(current: AgentId): Promise<AgentId[]> {
  const installed = await Promise.all(
    AGENT_IDS.map((id) =>
      id === current
        ? true
        : invoke<boolean>("check_app_installed", { editor: AGENT_EDITORS[id] }).catch(() => false)
    )
  );
  return AGENT_IDS.filter((_, index) => installed[index]);
}
