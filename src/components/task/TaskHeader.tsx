import { useShortcutLabels } from "../../shortcuts/runtime";
import { MouseEvent, ReactNode, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { BranchIcon, ChevronLeftIcon, PullRequestIcon, SidebarIcon } from "../ui/Icons";
import { GitStatus, IssueLinearInfo, Task } from "../../types";
import { Badge } from "../ui/Badge";
import { stateVariant } from "../worktree/cardStyles";
import { SessionStatus } from "../../hooks/useTerminalSession";
import { taskTransitionName } from "../../services/taskTransition";
import { PullRequestLinks } from "../worktree/PullRequestLinks";
import { EditableTaskTitle } from "./EditableTaskTitle";
import { linearIssueUrl } from "../../utils";

interface TaskHeaderProps {
  task: Task;
  linearInfo?: IssueLinearInfo;
  linearOrgUrlKey?: string | null;
  gitStatus?: GitStatus;
  sessionStatus: SessionStatus;
  sidebarCollapsed: boolean;
  onExpandSidebar: () => void;
  onBack: () => void;
  onCloseEditor?: () => void;
  closingEditor?: boolean;
  surfaceSwitcher?: ReactNode;
  onRename?: (title: string) => Promise<unknown>;
  onRenameError?: (message: string) => void;
}

const statusLabel: Record<SessionStatus["kind"], { text: string; dot: string }> = {
  connecting: { text: "connecting", dot: "bg-text-muted animate-pulse" },
  running: { text: "running", dot: "bg-success" },
  exited: { text: "ended", dot: "bg-text-muted" },
};

export function TaskHeader({
  task,
  linearInfo,
  linearOrgUrlKey,
  gitStatus,
  sessionStatus,
  sidebarCollapsed,
  onExpandSidebar,
  onBack,
  onCloseEditor,
  closingEditor,
  surfaceSwitcher,
  onRename,
  onRenameError,
}: TaskHeaderProps) {
  const [linkError, setLinkError] = useState<string | null>(null);
  const issueUrl = task.linearIssueIdentifier
    ? linearIssueUrl(task.linearIssueIdentifier, linearOrgUrlKey)
    : null;
  const openIssue = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    if (!issueUrl) return;
    setLinkError(null);
    openUrl(issueUrl).catch(() => setLinkError("Could not open Linear"));
  };
  const status = statusLabel[sessionStatus.kind];
  const issueStatus = linearInfo?.status ?? null;
  const prs = task.linearIssueId ? (linearInfo?.prs ?? []) : [];
  const label = useShortcutLabels();
  return (
    <div
      className="flex items-center gap-3 px-4 min-h-12 py-2 border-b border-border flex-shrink-0"
      style={{ viewTransitionName: taskTransitionName("shell") }}
      data-drag-region
    >
      {sidebarCollapsed && (
        <>
          <button
            type="button"
            onClick={onExpandSidebar}
            className="w-6 h-6 flex items-center justify-center flex-shrink-0 rounded-md text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
            title={`Expand sidebar (${label("app.sidebar")})`}
          >
            <SidebarIcon size={14} />
          </button>
          <span
            role="separator"
            aria-orientation="vertical"
            className="self-stretch -my-2 w-px bg-border"
          />
        </>
      )}
      <button
        type="button"
        onClick={onBack}
        aria-label={`Back to tasks (${label("native.back")})`}
        title={`Back to tasks (${label("native.back")})`}
        className="w-6 h-6 flex items-center justify-center flex-shrink-0 rounded-md text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
      >
        <ChevronLeftIcon size={14} />
      </button>
      {issueUrl && (
        <a
          href={issueUrl}
          title="Open issue in Linear"
          onClick={openIssue}
          onAuxClick={(event) => {
            if (event.button === 1) openIssue(event);
          }}
          className="flex-shrink-0 whitespace-nowrap text-xs font-mono text-accent hover:text-accent-hover hover:underline cursor-pointer"
          style={{ viewTransitionName: taskTransitionName("identifier") }}
        >
          {task.linearIssueIdentifier}
        </a>
      )}
      {linkError && (
        <span
          role="alert"
          className="flex-shrink-0 whitespace-nowrap text-xs text-danger select-text"
        >
          {linkError}
        </span>
      )}
      {issueStatus && (
        <span
          className="inline-flex flex-shrink-0 whitespace-nowrap"
          style={{ viewTransitionName: taskTransitionName("status") }}
        >
          <Badge variant={stateVariant[issueStatus.type] ?? "default"}>{issueStatus.name}</Badge>
        </span>
      )}
      <EditableTaskTitle
        title={task.linearIssueTitle ?? task.branchName}
        textClassName="text-sm font-semibold text-text-primary"
        titleStyle={{ viewTransitionName: taskTransitionName("title") }}
        onRename={onRename}
        onError={onRenameError}
      />
      <span
        className="flex items-center gap-1 shrink-[999] text-xs font-mono text-text-muted whitespace-nowrap"
        style={{ viewTransitionName: taskTransitionName("branch") }}
      >
        <BranchIcon className="flex-shrink-0" />
        <span className="truncate min-w-16 select-text" title={task.branchName}>
          {task.branchName}
        </span>
        {gitStatus && gitStatus.ahead > 0 && (
          <span className="text-success" title={`${gitStatus.ahead} ahead of main`}>
            ↑{gitStatus.ahead}
          </span>
        )}
        {gitStatus && gitStatus.behind > 0 && (
          <span className="text-warning" title={`${gitStatus.behind} behind main`}>
            ↓{gitStatus.behind}
          </span>
        )}
        {gitStatus?.dirty && (
          <span className="text-warning" title="Uncommitted changes">
            ●
          </span>
        )}
      </span>
      {prs.length > 0 && (
        <span
          className="flex items-center gap-1.5 text-xs min-w-0 overflow-hidden whitespace-nowrap"
          style={{ viewTransitionName: taskTransitionName("prs") }}
        >
          <PullRequestIcon size={12} className="text-text-muted flex-shrink-0" />
          <PullRequestLinks prs={prs} />
        </span>
      )}
      <span className="ml-auto flex items-center gap-1.5 text-xs text-text-muted flex-shrink-0 whitespace-nowrap">
        {surfaceSwitcher}
        {onCloseEditor && (
          <button
            type="button"
            onClick={onCloseEditor}
            disabled={closingEditor}
            className="mr-3 hover:text-text-primary cursor-pointer disabled:opacity-50"
          >
            {closingEditor ? "Closing…" : "Close editor"}
          </button>
        )}
        <span className={`w-1.5 h-1.5 rounded-full ${status.dot}`} />
        {status.text}
      </span>
    </div>
  );
}
