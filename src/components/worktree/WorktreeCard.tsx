import { useState, useEffect, useRef, memo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Badge } from "../ui/Badge";
import {
  SpinnerIcon,
  ExternalLinkIcon,
  WarningCircleIcon,
  BranchIcon,
  CopyIcon,
  PullRequestIcon,
  MoreVerticalIcon,
  TrashIcon,
} from "../ui/Icons";
import { Worktree, Repo, EditorApp, GitStatus, IssueLinearInfo } from "../../types";
import { timeAgo } from "../../utils";

interface WorktreeCardProps {
  worktree: Worktree;
  repo: Repo;
  onDelete: (worktreeId: string) => void;
  linearInfo?: IssueLinearInfo;
  gitStatus?: GitStatus;
  selected?: boolean;
  index?: number;
  editorApp?: EditorApp;
  onOpenError?: (msg: string) => void;
  onToast?: (msg: string) => void;
  requestDelete?: boolean;
  onRequestDeleteHandled?: () => void;
}

const stateVariant: Record<string, "success" | "warning" | "accent" | "danger" | "default"> = {
  started: "accent",
  unstarted: "default",
  completed: "success",
  canceled: "danger",
  backlog: "default",
  triage: "warning",
};

export const WorktreeCard = memo(function WorktreeCard({
  worktree,
  repo,
  onDelete,
  linearInfo,
  gitStatus,
  selected,
  index,
  editorApp = "cursor",
  onOpenError,
  onToast,
  requestDelete,
  onRequestDeleteHandled,
}: WorktreeCardProps) {
  const status = linearInfo?.status ?? null;
  const pr = linearInfo ? linearInfo.pr : undefined;
  const [deleting, setDeleting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (requestDelete) {
      setConfirmDelete(true);
      onRequestDeleteHandled?.();
    }
  }, [requestDelete, onRequestDeleteHandled]);

  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [menuOpen]);

  const handleOpen = async () => {
    try {
      await invoke<string>("open_editor", {
        editor: editorApp,
        path: worktree.path,
        branchName: worktree.branchName,
      });
    } catch (e) {
      const msg = typeof e === "string" ? e : `Failed to open ${editorApp}`;
      onOpenError?.(msg);
    }
  };

  const handleDeleteClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    setConfirmDelete(true);
  };

  const handleDeleteConfirm = async (e: React.MouseEvent) => {
    e.stopPropagation();
    setConfirmDelete(false);
    setDeleteError(null);
    setDeleting(true);
    try {
      await invoke<string>("git_worktree_remove", {
        repoPath: repo.localPath,
        worktreePath: worktree.path,
      });
      onDelete(worktree.id);
    } catch (e) {
      const msg = typeof e === "string" ? e : "Failed to remove worktree from disk";
      setDeleteError(msg);
      setDeleting(false);
    }
  };

  const handleForceRemove = (e: React.MouseEvent) => {
    e.stopPropagation();
    setDeleteError(null);
    onDelete(worktree.id);
  };

  const handleDeleteCancel = (e: React.MouseEvent) => {
    e.stopPropagation();
    setConfirmDelete(false);
    setDeleteError(null);
  };

  const handlePrClick = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (pr) {
      openUrl(pr.url);
    } else {
      try {
        const remoteUrl = await invoke<string>("git_remote_url", { repoPath: repo.localPath });
        openUrl(`${remoteUrl}/compare/${worktree.branchName}?expand=1`);
      } catch {
        console.error("Could not determine remote URL");
      }
    }
  };

  const handleMenuAction = async (action: string) => {
    setMenuOpen(false);
    switch (action) {
      case "copy-branch":
        navigator.clipboard.writeText(worktree.branchName);
        onToast?.("Branch name copied");
        break;
      case "copy-path":
        navigator.clipboard.writeText(worktree.path);
        onToast?.("Worktree path copied");
        break;
      case "open-claude-code":
        try {
          await invoke<string>("open_editor", {
            editor: "claude-code",
            path: worktree.path,
            branchName: worktree.branchName,
          });
        } catch (e) {
          const msg = typeof e === "string" ? e : "Failed to open Claude Code";
          onOpenError?.(msg);
        }
        break;
    }
  };

  const handleOpenLinear = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (worktree.linearIssueIdentifier) {
      openUrl(`https://linear.app/issue/${worktree.linearIssueIdentifier}`);
    }
  };

  const handleCopyPath = (e: React.MouseEvent) => {
    e.stopPropagation();
    navigator.clipboard.writeText(worktree.path);
    onToast?.("Worktree path copied");
  };

  const handleCopyBranch = (e: React.MouseEvent) => {
    e.stopPropagation();
    navigator.clipboard.writeText(worktree.branchName);
    onToast?.("Branch name copied");
  };

  const age = gitStatus ? timeAgo(gitStatus.last_commit_epoch) : null;

  return (
    <div
      onClick={handleOpen}
      className={`group bg-bg-secondary border rounded-xl p-4 hover:border-border-light hover:bg-bg-tertiary/50 transition-all cursor-pointer ${
        selected ? "border-accent/50 bg-bg-tertiary/30" : "border-border"
      }`}
    >
      <div className="flex items-start justify-between gap-3">
        {/* Index number */}
        {index !== undefined && index <= 9 && (
          <span className="text-xs font-mono text-text-muted/40 flex-shrink-0 w-4 pt-0.5 text-right">
            {index}
          </span>
        )}

        <div className="min-w-0 flex-1">
          {/* Row 1: identifier + status + age */}
          <div className="flex items-center gap-2 mb-1">
            {worktree.linearIssueIdentifier ? (
              <button
                onClick={handleOpenLinear}
                className="text-xs font-mono text-accent hover:text-accent-hover transition-colors flex-shrink-0 cursor-pointer inline-flex items-center gap-1"
                title="Open on Linear"
              >
                {worktree.linearIssueIdentifier}
                <ExternalLinkIcon size={10} />
              </button>
            ) : (
              <span className="text-xs font-mono text-text-muted flex-shrink-0">
                No issue
              </span>
            )}
            {status && (
              <Badge variant={stateVariant[status.type] ?? "default"}>{status.name}</Badge>
            )}
            {age && age.label && (
              <span
                className={`text-xs ml-auto flex-shrink-0 ${
                  age.veryStale ? "text-warning" : "text-text-muted"
                }`}
                title={`Last commit: ${age.label}`}
              >
                {age.veryStale && <WarningCircleIcon size={10} className="inline mr-1 -mt-0.5" />}
                {age.label}
              </span>
            )}
          </div>

          {/* Row 2: title */}
          <h3 className="text-sm font-medium text-text-primary truncate">
            {worktree.linearIssueTitle || worktree.branchName}
          </h3>

          {/* Row 3: branch + git status */}
          <div className="flex items-center gap-3 mt-2">
            <div className="flex items-center gap-1.5 min-w-0">
              <BranchIcon size={12} className="text-text-muted flex-shrink-0" />
              <button
                onClick={handleCopyPath}
                className="text-xs font-mono text-text-muted hover:text-text-primary truncate transition-colors cursor-pointer text-left"
                title="Copy worktree path"
              >
                {worktree.branchName}
              </button>
              <button
                onClick={handleCopyBranch}
                className="text-text-muted hover:text-text-primary transition-colors cursor-pointer"
                title="Copy branch name"
              >
                <CopyIcon size={12} />
              </button>
            </div>

            {gitStatus && (
              <div className="flex items-center gap-2 flex-shrink-0">
                {(gitStatus.ahead > 0 || gitStatus.behind > 0) && (
                  <span className="text-xs text-text-muted">
                    {gitStatus.ahead > 0 && (
                      <span className="text-success" title={`${gitStatus.ahead} ahead of main`}>
                        ↑{gitStatus.ahead}
                      </span>
                    )}
                    {gitStatus.ahead > 0 && gitStatus.behind > 0 && " "}
                    {gitStatus.behind > 0 && (
                      <span className="text-warning" title={`${gitStatus.behind} behind main`}>
                        ↓{gitStatus.behind}
                      </span>
                    )}
                  </span>
                )}
                {gitStatus.dirty && (
                  <span
                    className="w-2 h-2 rounded-full bg-warning flex-shrink-0"
                    title="Uncommitted changes"
                  />
                )}
              </div>
            )}
          </div>

          {/* Row 4: PR link / create PR — pointer-events-none container so empty space clicks pass through to card */}
          {pr !== undefined && (
            <div className="mt-2 text-left pointer-events-none">
              <span
                role="button"
                onClick={handlePrClick}
                className="pointer-events-auto inline items-baseline text-xs text-accent hover:text-accent-hover transition-colors cursor-pointer"
              >
                <PullRequestIcon size={12} className="inline -mt-0.5 mr-1.5" />
                {pr ? (
                  <>
                    PR #{pr.number}: {pr.title}{" "}
                    <Badge
                      variant={
                        pr.state === "open"
                          ? "success"
                          : pr.state === "merged"
                            ? "accent"
                            : "default"
                      }
                    >
                      {pr.state}
                    </Badge>
                  </>
                ) : (
                  <span className="text-text-muted hover:text-accent">Create PR</span>
                )}
              </span>
            </div>
          )}
        </div>

        {/* Right side: action buttons */}
        <div className="flex flex-col items-center gap-1 flex-shrink-0">
          {/* Context menu */}
          <div className="relative" ref={menuRef}>
            <button
              onClick={(e) => {
                e.stopPropagation();
                setMenuOpen(!menuOpen);
              }}
              className="w-8 h-8 flex items-center justify-center rounded-lg text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors opacity-0 group-hover:opacity-100 cursor-pointer"
              title="More actions"
            >
              <MoreVerticalIcon />
            </button>

            {menuOpen && (
              <div className="absolute right-0 top-9 z-50 w-48 rounded-lg border border-border bg-bg-secondary shadow-xl py-1">
                <MenuButton label="⌘B" onClick={() => handleMenuAction("copy-branch")}>
                  Copy branch name
                </MenuButton>
                <MenuButton label="⌘⇧C" onClick={() => handleMenuAction("copy-path")}>
                  Copy worktree path
                </MenuButton>
                <MenuButton onClick={() => handleMenuAction("open-claude-code")}>
                  Open in Claude Code
                </MenuButton>
              </div>
            )}
          </div>

          {/* Delete button */}
          <button
            onClick={handleDeleteClick}
            disabled={deleting}
            className="w-8 h-8 flex items-center justify-center rounded-lg text-text-muted hover:text-danger hover:bg-danger/10 transition-colors opacity-0 group-hover:opacity-100 cursor-pointer"
            title="Delete worktree (⌘D)"
          >
            {deleting ? <SpinnerIcon size={16} /> : <TrashIcon />}
          </button>
        </div>
      </div>

      {/* Inline delete confirmation */}
      {confirmDelete && (
        <div
          className="mt-3 flex items-center justify-between gap-3 pt-3 border-t border-border"
          onClick={(e) => e.stopPropagation()}
        >
          <span className="text-xs text-text-secondary truncate">
            Delete <strong className="text-text-primary">{worktree.linearIssueIdentifier || worktree.branchName}</strong>{" "}
            and remove from disk?
          </span>
          <div className="flex gap-2 flex-shrink-0">
            <button
              onClick={handleDeleteCancel}
              className="px-3 py-1 text-xs rounded-md text-text-secondary hover:text-text-primary hover:bg-bg-hover border border-border transition-colors cursor-pointer"
            >
              Cancel
            </button>
            <button
              onClick={handleDeleteConfirm}
              className="px-3 py-1 text-xs rounded-md text-white bg-danger hover:bg-danger-hover transition-colors cursor-pointer"
            >
              Delete
            </button>
          </div>
        </div>
      )}

      {/* Git removal failed — offer force-remove from app */}
      {deleteError && (
        <div
          className="mt-3 pt-3 border-t border-border space-y-2"
          onClick={(e) => e.stopPropagation()}
        >
          <p className="text-xs text-danger leading-relaxed">{deleteError}</p>
          <div className="flex items-center justify-between gap-3">
            <span className="text-xs text-text-secondary">Remove from app anyway?</span>
            <div className="flex gap-2 flex-shrink-0">
              <button
                onClick={handleDeleteCancel}
                className="px-3 py-1 text-xs rounded-md text-text-secondary hover:text-text-primary hover:bg-bg-hover border border-border transition-colors cursor-pointer"
              >
                Cancel
              </button>
              <button
                onClick={handleForceRemove}
                className="px-3 py-1 text-xs rounded-md text-white bg-danger hover:bg-danger-hover transition-colors cursor-pointer"
              >
                Remove
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
});

function MenuButton({
  onClick,
  children,
  label,
}: {
  onClick: () => void;
  children: React.ReactNode;
  label?: string;
}) {
  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className="w-full flex items-center justify-between px-3 py-1.5 text-xs text-text-secondary hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
    >
      <span>{children}</span>
      {label && <span className="text-text-muted font-mono text-[10px]">{label}</span>}
    </button>
  );
}
