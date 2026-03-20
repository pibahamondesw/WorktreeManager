import type { Dispatch, SetStateAction } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useKeyboardShortcuts } from "./useKeyboardShortcuts";
import { Worktree, Repo, EditorApp } from "../types";

interface Params {
  repo: Repo | undefined;
  worktrees: Worktree[];
  selectedWorktree: Worktree | null;
  editorApp: EditorApp;
  showNew: boolean;
  setShowNew: (v: boolean) => void;
  setSelectedIndex: Dispatch<SetStateAction<number>>;
  setDeleteRequested: (v: boolean) => void;
  handleRefresh: () => void;
  showToast: (msg: string) => void;
}

export function useWorktreeListKeyboardShortcuts({
  repo,
  worktrees,
  selectedWorktree,
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
      n: { handler: () => setShowNew(true), enabled: !!repo },
      r: { handler: () => handleRefresh(), enabled: !!repo },
      ArrowDown: {
        handler: () => setSelectedIndex((i) => Math.min(i + 1, worktrees.length - 1)),
        enabled: worktrees.length > 0,
      },
      j: {
        handler: () => setSelectedIndex((i) => Math.min(i + 1, worktrees.length - 1)),
        enabled: worktrees.length > 0,
      },
      ArrowUp: {
        handler: () => setSelectedIndex((i) => Math.max(i - 1, 0)),
        enabled: worktrees.length > 0,
      },
      k: {
        handler: () => setSelectedIndex((i) => Math.max(i - 1, 0)),
        enabled: worktrees.length > 0,
      },
      Enter: {
        handler: () => {
          if (selectedWorktree) {
            invoke<string>("open_editor", {
              editor: editorApp,
              path: selectedWorktree.path,
            }).catch((err) =>
              showToast(typeof err === "string" ? err : `Failed to open ${editorApp}`)
            );
          }
        },
        enabled: !!selectedWorktree,
      },
      Escape: { handler: () => setSelectedIndex(-1) },
      l: {
        handler: () => {
          if (selectedWorktree?.linearIssueIdentifier) {
            openUrl(`https://linear.app/issue/${selectedWorktree.linearIssueIdentifier}`);
          }
        },
        enabled: !!selectedWorktree?.linearIssueIdentifier,
      },
      "meta+d": {
        handler: () => setDeleteRequested(true),
        enabled: !!selectedWorktree && !!repo,
      },
      "meta+b": {
        handler: () => {
          if (selectedWorktree) {
            navigator.clipboard.writeText(selectedWorktree.branchName);
            showToast("Branch name copied");
          }
        },
        enabled: !!selectedWorktree,
      },
      "meta+shift+c": {
        handler: () => {
          if (selectedWorktree) {
            navigator.clipboard.writeText(selectedWorktree.path);
            showToast("Path copied");
          }
        },
        enabled: !!selectedWorktree,
      },
      ...Object.fromEntries(
        Array.from({ length: 9 }, (_, i) => [
          String(i + 1),
          { handler: () => setSelectedIndex(i), enabled: i < worktrees.length },
        ])
      ),
    },
    { enabled: !showNew }
  );
}
