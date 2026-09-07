import { BranchIcon, ChevronLeftIcon, SidebarIcon } from "../ui/Icons";
import { Task } from "../../types";
import { SessionStatus } from "../../hooks/useTerminalSession";

interface TaskHeaderProps {
  task: Task;
  sessionStatus: SessionStatus;
  sidebarCollapsed: boolean;
  onExpandSidebar: () => void;
  onBack: () => void;
}

const statusLabel: Record<SessionStatus["kind"], { text: string; dot: string }> = {
  connecting: { text: "connecting", dot: "bg-text-muted animate-pulse" },
  running: { text: "running", dot: "bg-success" },
  exited: { text: "ended", dot: "bg-text-muted" },
};

export function TaskHeader({
  task,
  sessionStatus,
  sidebarCollapsed,
  onExpandSidebar,
  onBack,
}: TaskHeaderProps) {
  const status = statusLabel[sessionStatus.kind];
  return (
    <div
      className="flex items-center gap-3 px-4 min-h-12 py-2 border-b border-border flex-shrink-0"
      data-drag-region
    >
      <button
        type="button"
        onClick={onBack}
        aria-label="Back to tasks (⌘[)"
        title="Back to tasks (⌘[)"
        className="w-6 h-6 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
      >
        <ChevronLeftIcon size={14} />
      </button>
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
      {task.linearIssueIdentifier && (
        <span className="text-xs font-mono text-text-muted">{task.linearIssueIdentifier}</span>
      )}
      <h3 className="text-sm font-semibold text-text-primary truncate min-w-0">
        {task.linearIssueTitle ?? task.branchName}
      </h3>
      <span className="flex items-center gap-1 text-xs font-mono text-text-muted truncate">
        <BranchIcon />
        {task.branchName}
      </span>
      <span className="ml-auto flex items-center gap-1.5 text-xs text-text-muted flex-shrink-0">
        <span className={`w-1.5 h-1.5 rounded-full ${status.dot}`} />
        {status.text}
      </span>
    </div>
  );
}
