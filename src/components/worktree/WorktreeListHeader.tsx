import { Button } from "../ui/Button";
import { EditorPicker } from "../ui/EditorPicker";
import { RefreshIcon, PlusIcon, SearchIcon } from "../ui/Icons";
import { EditorApp } from "../../types";

interface WorktreeListHeaderProps {
  workspaceName: string;
  taskCount: number;
  repoCount: number;
  editorApp: EditorApp;
  onEditorChange: (editor: EditorApp) => void;
  onRefresh: () => void;
  refreshing: boolean;
  onNewTask: () => void;
  onOpenSearch: () => void;
}

export function WorktreeListHeader({
  workspaceName,
  taskCount,
  repoCount,
  editorApp,
  onEditorChange,
  onRefresh,
  refreshing,
  onNewTask,
  onOpenSearch,
}: WorktreeListHeaderProps) {
  return (
    <>
      <div className="h-[32px] flex-shrink-0" data-drag-region />
      <div
        className="flex items-center justify-between px-6 h-12 border-b border-border flex-shrink-0"
        data-drag-region
      >
        <div className="flex items-center gap-3">
          <h2 className="text-sm font-semibold text-text-primary">
            {workspaceName}
          </h2>
          <span className="text-xs text-text-muted">
            {taskCount} task{taskCount !== 1 ? "s" : ""}
            {repoCount > 1 ? ` · ${repoCount} repos` : ""}
          </span>
          <button
            type="button"
            onClick={onOpenSearch}
            className="w-6 h-6 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
            title="Search tasks (⌘K — ⌘F for this workspace)"
          >
            <SearchIcon />
          </button>
        </div>
        <div className="flex items-center gap-2">
          <EditorPicker value={editorApp} onChange={onEditorChange} />
          <button
            type="button"
            onClick={onRefresh}
            className="w-8 h-8 flex items-center justify-center rounded-lg text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
            title="Refresh Linear info (⌘R)"
          >
            <RefreshIcon className={refreshing ? "animate-spin" : ""} />
          </button>
          <Button onClick={onNewTask} className="h-8 text-xs">
            <PlusIcon />
            New Task
            <kbd className="ml-1 text-[10px] opacity-50 font-mono">N</kbd>
          </Button>
        </div>
      </div>
    </>
  );
}
