import { invoke } from "@tauri-apps/api/core";
import { EditorApp } from "../types";

interface OpenEditorHandlers {
  /** A hint the user must act on (e.g. how to start the Claude task in Zed, or repos an editor couldn't open natively). */
  onMessage?: (msg: string) => void;
  /** Failure to launch the editor. */
  onError?: (msg: string) => void;
}

export interface OpenEditorResult {
  message: string;
  /** Absolute path of the generated .code-workspace, when one was used (Cursor). */
  workspaceFile: string | null;
}

/**
 * Open a task's folders in the given editor via the `open_editor` Tauri command.
 *
 * `folders` is the ordered set of included member paths; `folders[0]` is only the working
 * directory for CLI launches and carries no priority (there is no "main" repo). For Cursor
 * with more than one folder the backend manages a `.code-workspace` keyed by the
 * stable task ID. Any hint the backend returns (Zed manual task, repos an
 * editor couldn't open natively) is surfaced through `onMessage`.
 */
export async function openEditorForWorktree(
  editor: EditorApp,
  folders: string[],
  taskId: string,
  legacyWorkspaceFilePath: string | null | undefined,
  { onMessage, onError }: OpenEditorHandlers = {}
): Promise<OpenEditorResult | null> {
  try {
    const result = await invoke<OpenEditorResult>("open_editor", {
      editor,
      folders,
      taskId,
      legacyWorkspaceFilePath,
    });
    if (result?.message) onMessage?.(result.message);
    return result;
  } catch (e) {
    onError?.(typeof e === "string" ? e : `Failed to open ${editor}`);
    return null;
  }
}
