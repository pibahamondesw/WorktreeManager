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

/** The single place that decides whether an editor choice opens a task inside the app. */
export function taskSurfaceFor(
  editor: EditorApp,
  views: AgentViews = DEFAULT_AGENT_VIEWS
): TaskSurface {
  if (editor === "vscode-web") return { kind: "editor" };
  const agent = EDITOR_AGENTS[editor];
  return agent ? agentSurface(agent, views) : { kind: "external" };
}

export const isEmbedded = (surface: TaskSurface): boolean => surface.kind !== "external";

export const surfaceAgent = (surface: TaskSurface): AgentId | null =>
  surface.kind === "terminal" || surface.kind === "chat" ? surface.agent : null;
