import { invoke } from "@tauri-apps/api/core";
import { EditorApp } from "../types";

interface OpenEditorHandlers {
  /** A hint the user must act on (e.g. how to start the Claude task in Zed). */
  onMessage?: (msg: string) => void;
  /** Failure to launch the editor. */
  onError?: (msg: string) => void;
}

/**
 * Open a worktree in the given editor via the `open_editor` Tauri command.
 *
 * Most editors launch silently; "Zed + Claude" can't auto-run its Claude task, so the
 * backend returns an instruction string that we surface through `onMessage`.
 */
export async function openEditorForWorktree(
  editor: EditorApp,
  path: string,
  branchName: string | undefined,
  { onMessage, onError }: OpenEditorHandlers = {}
): Promise<void> {
  try {
    const message = await invoke<string>("open_editor", { editor, path, branchName });
    if (editor === "zed-claude" && message) onMessage?.(message);
  } catch (e) {
    onError?.(typeof e === "string" ? e : `Failed to open ${editor}`);
  }
}
