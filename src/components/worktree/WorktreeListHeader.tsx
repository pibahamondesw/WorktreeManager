import { Button } from "../ui/Button";
import { EditorPicker } from "../ui/EditorPicker";
import {
  RefreshIcon,
  PlusIcon,
  SearchIcon,
  SidebarIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
} from "../ui/Icons";
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
  sidebarCollapsed: boolean;
  onExpandSidebar: () => void;
  canGoBack: boolean;
  canGoForward: boolean;
  onGoBack: () => void;
  onGoForward: () => void;
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
  sidebarCollapsed,
  onExpandSidebar,
  canGoBack,
  canGoForward,
  onGoBack,
  onGoForward,
}: WorktreeListHeaderProps) {
  return (
    <>
      <div
        className="flex items-center justify-between flex-wrap gap-2 px-6 min-h-12 py-2 border-b border-border flex-shrink-0"
        data-drag-region
      >
        <div className="flex items-center gap-3 min-w-0 flex-wrap">
          <div className="flex items-center gap-0.5">
            <HistoryButton title="Back (⌘←)" disabled={!canGoBack} onClick={onGoBack}>
              <ChevronLeftIcon size={14} />
            </HistoryButton>
            <HistoryButton title="Forward (⌘→)" disabled={!canGoForward} onClick={onGoForward}>
              <ChevronRightIcon size={14} />
            </HistoryButton>
          </div>
          {sidebarCollapsed && (
            <button
              type="button"
              onClick={onExpandSidebar}
              className="w-6 h-6 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
              title="Expand sidebar ["
            >
              <SidebarIcon size={14} />
            </button>
          )}
          <h2 className="text-sm font-semibold text-text-primary">{workspaceName}</h2>
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
            <kbd className="ml-1 text-[0.625rem] opacity-50 font-mono">N</kbd>
          </Button>
        </div>
      </div>
    </>
  );
}

function HistoryButton({
  title,
  disabled,
  onClick,
  children,
}: {
  title: string;
  disabled: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={title}
      title={title}
      className="w-6 h-6 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-default disabled:hover:text-text-muted disabled:hover:bg-transparent"
    >
      {children}
    </button>
  );
}
