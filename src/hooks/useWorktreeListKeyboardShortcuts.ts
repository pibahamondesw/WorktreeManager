import type { Dispatch, SetStateAction } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useKeyboardShortcuts } from "./useKeyboardShortcuts";
import { Task, VaultConfig, Workspace } from "../types";
import { OpenTaskOptions } from "./useOpenTask";
import { ensureTaskNote, taskNoteUri } from "../services/notes";
import { linearIssueUrl } from "../utils";

interface Params {
  workspace: Workspace | undefined;
  vault: VaultConfig;
  tasks: Task[];
  selectedTask: Task | null;
  showNew: boolean;
  searchOpen: boolean;
  setShowNew: (v: boolean) => void;
  setSelectedIndex: Dispatch<SetStateAction<number>>;
  setDeleteRequested: (v: boolean) => void;
  handleRefresh: () => void;
  showToast: (msg: string) => void;
  onOpenTask: (task: Task, options?: OpenTaskOptions) => Promise<void>;
  taskOpen: boolean;
}

export function useWorktreeListKeyboardShortcuts({
  workspace,
  vault,
  tasks,
  selectedTask,
  showNew,
  searchOpen,
  setShowNew,
  setSelectedIndex,
  setDeleteRequested,
  handleRefresh,
  showToast,
  onOpenTask,
  taskOpen,
}: Params): void {
  useKeyboardShortcuts(
    {
      n: { handler: () => setShowNew(true), enabled: !!workspace },
      "meta+r": { handler: () => handleRefresh(), enabled: !!workspace },
      ArrowDown: {
        handler: () => setSelectedIndex((i) => Math.min(i + 1, tasks.length - 1)),
        enabled: tasks.length > 0,
      },
      j: {
        handler: () => setSelectedIndex((i) => Math.min(i + 1, tasks.length - 1)),
        enabled: tasks.length > 0,
      },
      ArrowUp: {
        handler: () => setSelectedIndex((i) => Math.max(i - 1, 0)),
        enabled: tasks.length > 0,
      },
      k: {
        handler: () => setSelectedIndex((i) => Math.max(i - 1, 0)),
        enabled: tasks.length > 0,
      },
      Enter: {
        handler: () => {
          if (selectedTask) {
            void onOpenTask(selectedTask, { onMessage: showToast, onError: showToast });
          }
        },
        enabled: !!selectedTask,
      },
      Escape: { handler: () => setSelectedIndex(-1) },
      l: {
        handler: () => {
          if (selectedTask?.linearIssueIdentifier) {
            openUrl(linearIssueUrl(selectedTask.linearIssueIdentifier, workspace?.linearOrgUrlKey));
          }
        },
        enabled: !!selectedTask?.linearIssueIdentifier,
      },
      o: {
        handler: () => {
          if (!selectedTask || !workspace) return;
          void ensureTaskNote(vault, workspace, selectedTask).then((notePath) => {
            if (notePath) {
              openUrl(taskNoteUri(notePath)).catch(() =>
                showToast("Could not open the note in Obsidian")
              );
            } else showToast("Could not open the task note");
          });
        },
        enabled: !!selectedTask && !!workspace && vault.enabled,
      },
      "meta+d": {
        handler: () => setDeleteRequested(true),
        enabled: !!selectedTask && !!workspace,
      },
      "meta+b": {
        handler: () => {
          if (selectedTask) {
            navigator.clipboard.writeText(selectedTask.branchName);
            showToast("Branch name copied");
          }
        },
        enabled: !!selectedTask,
      },
      "meta+shift+c": {
        handler: () => {
          if (selectedTask) {
            navigator.clipboard.writeText(selectedTask.members.map((m) => m.path).join("\n"));
            showToast("Path copied");
          }
        },
        enabled: !!selectedTask,
      },
      ...Object.fromEntries(
        Array.from({ length: 10 }, (_, i) => [
          String(i),
          { handler: () => setSelectedIndex(i), enabled: i < tasks.length },
        ])
      ),
    },
    { enabled: !showNew && !searchOpen && !taskOpen }
  );
}
