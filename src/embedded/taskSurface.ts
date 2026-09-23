import { AgentId, EditorApp, TaskSurface } from "../types";

export const CLAUDE_TERMINAL: TaskSurface = { kind: "terminal", agent: "claude" };
export const CODEX_TERMINAL: TaskSurface = { kind: "terminal", agent: "codex" };

const AGENT_SURFACES: Partial<Record<EditorApp, TaskSurface>> = {
  "claude-code": CLAUDE_TERMINAL,
  codex: CODEX_TERMINAL,
  "claude-chat": { kind: "chat", agent: "claude" },
  "codex-chat": { kind: "chat", agent: "codex" },
};

/** The single place that decides whether an editor choice opens a task inside the app. */
export function taskSurfaceFor(editor: EditorApp): TaskSurface {
  if (editor === "vscode-web") return { kind: "editor" };
  return AGENT_SURFACES[editor] ?? { kind: "external" };
}

export const isEmbedded = (surface: TaskSurface): boolean => surface.kind !== "external";

export const surfaceAgent = (surface: TaskSurface): AgentId | null =>
  surface.kind === "terminal" || surface.kind === "chat" ? surface.agent : null;
