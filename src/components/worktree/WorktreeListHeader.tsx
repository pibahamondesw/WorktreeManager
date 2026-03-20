import { Button } from "../ui/Button";
import { EditorPicker } from "../ui/EditorPicker";
import { RefreshIcon, PlusIcon } from "../ui/Icons";
import { Repo, EditorApp } from "../../types";

interface WorktreeListHeaderProps {
  repo: Repo;
  worktreeCount: number;
  editorApp: EditorApp;
  onEditorChange: (editor: EditorApp) => void;
  onRefresh: () => void;
  refreshing: boolean;
  onNewWorktree: () => void;
}

export function WorktreeListHeader({
  repo,
  worktreeCount,
  editorApp,
  onEditorChange,
  onRefresh,
  refreshing,
  onNewWorktree,
}: WorktreeListHeaderProps) {
  return (
    <>
      <div className="h-[38px] flex-shrink-0" data-tauri-drag-region />
      <div
        className="flex items-center justify-between px-6 h-12 border-b border-border flex-shrink-0"
        data-tauri-drag-region
      >
        <div className="flex items-center gap-3" data-tauri-drag-region>
          <h2 className="text-sm font-semibold text-text-primary" data-tauri-drag-region>
            {repo.name}
          </h2>
          <span className="text-xs text-text-muted">
            {worktreeCount} worktree{worktreeCount !== 1 ? "s" : ""}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <EditorPicker value={editorApp} onChange={onEditorChange} />
          <button
            type="button"
            onClick={onRefresh}
            className="w-8 h-8 flex items-center justify-center rounded-lg text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
            title="Refresh Linear info (R)"
          >
            <RefreshIcon className={refreshing ? "animate-spin" : ""} />
          </button>
          <Button onClick={onNewWorktree} className="h-8 text-xs">
            <PlusIcon />
            New Worktree
            <kbd className="ml-1 text-[10px] opacity-50 font-mono">N</kbd>
          </Button>
        </div>
      </div>
    </>
  );
}
