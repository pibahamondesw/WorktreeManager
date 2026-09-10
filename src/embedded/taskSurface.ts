import { EditorApp, TaskSurface } from "../types";

export const CLAUDE_TERMINAL: TaskSurface = { kind: "terminal", agent: "claude" };

/** The single place that decides whether an editor choice opens a task inside the app. */
export function taskSurfaceFor(editor: EditorApp): TaskSurface {
  if (editor === "vscode-web") return { kind: "editor" };
  return editor === "claude-code" ? CLAUDE_TERMINAL : { kind: "external" };
}

export const isEmbedded = (surface: TaskSurface): boolean => surface.kind !== "external";
