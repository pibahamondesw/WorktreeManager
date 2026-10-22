import { AgentId, AgentViews, DEFAULT_AGENT_VIEWS, EditorApp, TaskSurface } from "../types";

const EDITOR_AGENTS: Partial<Record<EditorApp, AgentId>> = {
  "claude-code": "claude",
  codex: "codex",
};

export const agentSurface = (
  agent: AgentId,
  views: AgentViews = DEFAULT_AGENT_VIEWS
): TaskSurface => ({
  kind: views[agent],
  agent,
});

/**
 * The single place that decides whether an editor choice opens a task inside the app. An agent
 * choice reopens the agent the task last used.
 */
export function taskSurfaceFor(
  editor: EditorApp,
  views: AgentViews = DEFAULT_AGENT_VIEWS,
  taskAgent?: AgentId
): TaskSurface {
  if (editor === "vscode-web") return { kind: "editor" };
  if (editor === "opencode-web") return { kind: "opencode" };
  const agent = EDITOR_AGENTS[editor];
  return agent ? agentSurface(taskAgent ?? agent, views) : { kind: "external" };
}

export const isEmbedded = (surface: TaskSurface): boolean => surface.kind !== "external";
