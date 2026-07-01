import type { Dispatch, SetStateAction } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useKeyboardShortcuts } from "./useKeyboardShortcuts";
import { Task, Workspace, EditorApp } from "../types";
import { openEditorForWorktree } from "../services/openEditor";

interface Params {
  workspace: Workspace | undefined;
  tasks: Task[];
  selectedTask: Task | null;
  editorApp: EditorApp;
  showNew: boolean;
  setShowNew: (v: boolean) => void;
  setSelectedIndex: Dispatch<SetStateAction<number>>;
  setDeleteRequested: (v: boolean) => void;
  handleRefresh: () => void;
  showToast: (msg: string) => void;
}

export function useWorktreeListKeyboardShortcuts({
  workspace,
  tasks,
  selectedTask,
  editorApp,
  showNew,
  setShowNew,
  setSelectedIndex,
  setDeleteRequested,
  handleRefresh,
  showToast,
}: Params): void {
  useKeyboardShortcuts(
    {
      n: { handler: () => setShowNew(true), enabled: !!workspace },
      r: { handler: () => handleRefresh(), enabled: !!workspace },
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
            void openEditorForWorktree(
              editorApp,
              selectedTask.members.map((m) => m.path),
              selectedTask.branchName,
              workspace?.name,
              { onMessage: showToast, onError: showToast }
            );
          }
        },
        enabled: !!selectedTask,
      },
      Escape: { handler: () => setSelectedIndex(-1) },
      l: {
        handler: () => {
          if (selectedTask?.linearIssueIdentifier) {
            openUrl(`https://linear.app/issue/${selectedTask.linearIssueIdentifier}`);
          }
        },
        enabled: !!selectedTask?.linearIssueIdentifier,
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
        Array.from({ length: 9 }, (_, i) => [
          String(i + 1),
          { handler: () => setSelectedIndex(i), enabled: i < tasks.length },
        ])
      ),
    },
    { enabled: !showNew }
  );
}
