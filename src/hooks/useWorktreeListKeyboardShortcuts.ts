import type { Dispatch, SetStateAction } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useShortcutActions } from "../shortcuts/runtime";
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
  jumpToIndex: (index: number) => void;
  setDeleteRequested: (v: boolean) => void;
  handleRefresh: () => void;
  showToast: (msg: string) => void;
  onOpenTask: (task: Task, options?: OpenTaskOptions) => Promise<boolean>;
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
  jumpToIndex,
  setDeleteRequested,
  handleRefresh,
  showToast,
  onOpenTask,
  taskOpen,
}: Params): void {
  useShortcutActions(
    {
      "list.new": { handler: () => setShowNew(true), enabled: !!workspace },
      "list.refresh": { handler: () => handleRefresh(), enabled: !!workspace },
      "list.down": {
        handler: () => setSelectedIndex((i) => Math.min(i + 1, tasks.length - 1)),
        enabled: tasks.length > 0,
      },
      "list.next": {
        handler: () => setSelectedIndex((i) => Math.min(i + 1, tasks.length - 1)),
        enabled: tasks.length > 0,
      },
      "list.up": {
        handler: () => setSelectedIndex((i) => Math.max(i - 1, 0)),
        enabled: tasks.length > 0,
      },
      "list.previous": {
        handler: () => setSelectedIndex((i) => Math.max(i - 1, 0)),
        enabled: tasks.length > 0,
      },
      "list.open": {
        handler: () => {
          if (selectedTask) {
            void onOpenTask(selectedTask, { onMessage: showToast, onError: showToast });
          }
        },
        enabled: !!selectedTask,
      },
      "list.clear": { handler: () => setSelectedIndex(-1) },
      "list.linear": {
        handler: () => {
          if (selectedTask?.linearIssueIdentifier) {
            openUrl(
              linearIssueUrl(selectedTask.linearIssueIdentifier, workspace?.linearOrgUrlKey)
            ).catch(() => showToast("Could not open Linear"));
          }
        },
        enabled: !!selectedTask?.linearIssueIdentifier,
      },
      "list.note": {
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
      "list.delete": {
        handler: () => setDeleteRequested(true),
        enabled: !!selectedTask && !!workspace,
      },
      "list.branch": {
        handler: () => {
          if (selectedTask) {
            navigator.clipboard
              .writeText(selectedTask.branchName)
              .then(() => showToast("Branch name copied"))
              .catch(() => showToast("Could not copy to clipboard"));
          }
        },
        enabled: !!selectedTask,
      },
      "list.path": {
        handler: () => {
          if (selectedTask) {
            navigator.clipboard
              .writeText(selectedTask.members.map((m) => m.path).join("\n"))
              .then(() => showToast("Path copied"))
              .catch(() => showToast("Could not copy to clipboard"));
          }
        },
        enabled: !!selectedTask,
      },
      ...Object.fromEntries(
        Array.from({ length: 10 }, (_, i) => [
          `list.jump.${i}`,
          { handler: () => jumpToIndex(i), enabled: i < tasks.length },
        ])
      ),
    },
    !showNew && !searchOpen && !taskOpen
  );
}
