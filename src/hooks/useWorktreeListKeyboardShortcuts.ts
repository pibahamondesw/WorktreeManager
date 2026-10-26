import type { Dispatch, SetStateAction } from "react";
import { useShortcutActions } from "../shortcuts/runtime";
import { Task, VaultConfig, Workspace } from "../types";
import { OpenTaskOptions } from "./useOpenTask";
import { runTaskAction, TaskActionId } from "../services/taskActions";

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
  const runSelectedTaskAction = (action: TaskActionId) => {
    if (selectedTask)
      void runTaskAction(action, { task: selectedTask, workspace, vault }, showToast);
  };

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
        handler: () => runSelectedTaskAction("open-linear"),
        enabled: !!selectedTask?.linearIssueIdentifier,
      },
      "list.note": {
        handler: () => runSelectedTaskAction("open-note"),
        enabled: !!selectedTask && !!workspace && vault.enabled,
      },
      "list.delete": {
        handler: () => setDeleteRequested(true),
        enabled: !!selectedTask && !!workspace,
      },
      "list.branch": {
        handler: () => runSelectedTaskAction("copy-branch"),
        enabled: !!selectedTask,
      },
      "list.path": {
        handler: () => runSelectedTaskAction("copy-path"),
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
