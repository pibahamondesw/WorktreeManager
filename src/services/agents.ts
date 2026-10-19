import { AgentId } from "../types";

/** What the UI shows about each CLI agent; behaviour lives in the backend agent adapters. */
export interface AgentCapabilities {
  label: string;
  productName: string;
  planUrl?: string;
  /** Usage windows shown on the compact indicator; every window when omitted. */
  primaryUsageWindows?: string[];
  alertHooksConfig: string;
  alertHooksTrust?: string;
}

export const AGENTS: Record<AgentId, AgentCapabilities> = {
  claude: {
    label: "Claude",
    productName: "Claude Code",
    primaryUsageWindows: ["five_hour", "seven_day"],
    alertHooksConfig: "~/.claude/settings.json",
  },
  codex: {
    label: "Codex",
    productName: "Codex",
    planUrl: "https://learn.chatgpt.com/docs/pricing",
    alertHooksConfig: "~/.codex/hooks.json",
    alertHooksTrust:
      "Run /hooks in the Codex chat or CLI to review and trust new or changed hooks.",
  },
};

export const AGENT_IDS = Object.keys(AGENTS) as AgentId[];
