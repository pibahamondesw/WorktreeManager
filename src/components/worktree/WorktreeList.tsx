import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { WorktreeCard } from "./WorktreeCard";
import { NewWorktreeModal } from "./NewWorktreeModal";
import { Button } from "../ui/Button";
import { EditorPicker } from "../ui/EditorPicker";
import { FolderIcon, RefreshIcon, PlusIcon, CodeBranchIcon } from "../ui/Icons";
import { useLinear } from "../../contexts/LinearContext";
import { useKeyboardShortcuts } from "../../hooks/useKeyboardShortcuts";
import { Worktree, Repo, EditorApp, GitStatus, IssueLinearInfo } from "../../types";

interface WorktreeListProps {
  worktrees: Worktree[];
  repo: Repo | undefined;
  onWorktreeCreated: (worktree: Worktree) => void;
  onWorktreeDeleted: (worktreeId: string) => void;
  editorApp: EditorApp;
  onEditorChange: (editor: EditorApp) => void;
}

export function WorktreeList({
  worktrees,
  repo,
  onWorktreeCreated,
  onWorktreeDeleted,
  editorApp,
  onEditorChange,
}: WorktreeListProps) {
  const [showNew, setShowNew] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const [deleteRequested, setDeleteRequested] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [linearInfo, setLinearInfo] = useState<Record<string, IssueLinearInfo>>({});
  const [gitStatuses, setGitStatuses] = useState<Record<string, GitStatus>>({});
  const linear = useLinear();

  const toastTimer = useRef<number>(undefined!);

  useEffect(() => {
    return () => window.clearTimeout(toastTimer.current);
  }, []);

  const showToast = (msg: string) => {
    window.clearTimeout(toastTimer.current);
    setToast(msg);
    toastTimer.current = window.setTimeout(() => setToast(null), msg.length > 40 ? 4000 : 1500);
  };

  const fetchLinearInfo = useCallback(async () => {
    const issueIds = worktrees
      .map((wt) => wt.linearIssueId)
      .filter((id): id is string => !!id);
    if (issueIds.length === 0 || !linear) {
      setLinearInfo({});
      return;
    }
    const info = await linear.fetchIssueLinearInfoBatch(issueIds);
    setLinearInfo(info);
  }, [worktrees, linear]);

  const fetchGitStatuses = useCallback(async () => {
    if (worktrees.length === 0 || !repo) {
      setGitStatuses({});
      return;
    }
    try {
      const statuses = await invoke<Record<string, GitStatus>>("git_worktree_status_batch", {
        worktreePaths: worktrees.map((wt) => wt.path),
        repoPath: repo.localPath,
      });
      setGitStatuses(statuses);
    } catch {
      /* git status is best-effort */
    }
  }, [worktrees, repo]);

  const handleRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await Promise.all([fetchLinearInfo(), fetchGitStatuses()]);
    } finally {
      setRefreshing(false);
    }
  }, [fetchLinearInfo, fetchGitStatuses]);

  useEffect(() => {
    setSelectedIndex(-1);
  }, [repo?.id]);

  useEffect(() => { fetchLinearInfo(); }, [fetchLinearInfo]);
  useEffect(() => { fetchGitStatuses(); }, [fetchGitStatuses]);

  const selectedWorktree =
    selectedIndex >= 0 && selectedIndex < worktrees.length ? worktrees[selectedIndex] : null;

  useKeyboardShortcuts(
    {
      n: { handler: () => setShowNew(true), enabled: !!repo },
      r: { handler: () => handleRefresh(), enabled: !!repo },
      ArrowDown: { handler: () => setSelectedIndex((i) => Math.min(i + 1, worktrees.length - 1)), enabled: worktrees.length > 0 },
      j: { handler: () => setSelectedIndex((i) => Math.min(i + 1, worktrees.length - 1)), enabled: worktrees.length > 0 },
      ArrowUp: { handler: () => setSelectedIndex((i) => Math.max(i - 1, 0)), enabled: worktrees.length > 0 },
      k: { handler: () => setSelectedIndex((i) => Math.max(i - 1, 0)), enabled: worktrees.length > 0 },
      Enter: {
        handler: () => {
          if (selectedWorktree) {
            invoke<string>("open_editor", { editor: editorApp, path: selectedWorktree.path }).catch(
              (err) => showToast(typeof err === "string" ? err : `Failed to open ${editorApp}`)
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
      "meta+d": { handler: () => setDeleteRequested(true), enabled: !!selectedWorktree && !!repo },
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

  if (!repo) {
    return (
      <div className="flex-1 flex items-center justify-center" data-tauri-drag-region>
        <div className="text-center">
          <div className="w-12 h-12 rounded-xl bg-bg-tertiary flex items-center justify-center mx-auto mb-3">
            <FolderIcon className="text-text-muted" />
          </div>
          <p className="text-sm text-text-muted">Select a project or add one to get started</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 flex flex-col min-h-0">
      {/* Titlebar drag area */}
      <div className="h-[38px] flex-shrink-0" data-tauri-drag-region />
      {/* Header — fixed h-12 to match sidebar */}
      <div
        className="flex items-center justify-between px-6 h-12 border-b border-border flex-shrink-0"
        data-tauri-drag-region
      >
        <div className="flex items-center gap-3" data-tauri-drag-region>
          <h2 className="text-sm font-semibold text-text-primary" data-tauri-drag-region>
            {repo.name}
          </h2>
          <span className="text-xs text-text-muted">
            {worktrees.length} worktree{worktrees.length !== 1 ? "s" : ""}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <EditorPicker value={editorApp} onChange={onEditorChange} />
          <button
            onClick={handleRefresh}
            className="w-8 h-8 flex items-center justify-center rounded-lg text-text-muted hover:text-text-primary hover:bg-bg-hover transition-colors cursor-pointer"
            title="Refresh Linear info (R)"
          >
            <RefreshIcon className={refreshing ? "animate-spin" : ""} />
          </button>
          <Button onClick={() => setShowNew(true)} className="h-8 text-xs">
            <PlusIcon />
            New Worktree
            <kbd className="ml-1 text-[10px] opacity-50 font-mono">N</kbd>
          </Button>
        </div>
      </div>

      {/* Worktree cards */}
      <div className="flex-1 overflow-y-auto p-6">
        {worktrees.length === 0 ? (
          <div className="flex items-center justify-center h-full">
            <div className="text-center">
              <div className="w-12 h-12 rounded-xl bg-bg-tertiary flex items-center justify-center mx-auto mb-3">
                <CodeBranchIcon className="text-text-muted" />
              </div>
              <p className="text-sm text-text-muted mb-3">No active worktrees</p>
              <Button onClick={() => setShowNew(true)} className="text-xs">
                Create your first worktree
              </Button>
            </div>
          </div>
        ) : (
          <div className="grid gap-3">
            {worktrees.map((wt, i) => (
              <WorktreeCard
                key={wt.id}
                worktree={wt}
                repo={repo}
                onDelete={onWorktreeDeleted}
                linearInfo={wt.linearIssueId ? linearInfo[wt.linearIssueId] : undefined}
                gitStatus={gitStatuses[wt.path]}
                selected={i === selectedIndex}
                index={i + 1}
                editorApp={editorApp}
                onOpenError={showToast}
                onToast={showToast}
                requestDelete={i === selectedIndex && deleteRequested}
                onRequestDeleteHandled={() => setDeleteRequested(false)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Keyboard hints */}
      {worktrees.length > 0 && (
        <div className="flex-shrink-0 px-6 py-2 border-t border-border">
          <div className="flex items-center gap-3 text-[10px] text-text-muted font-mono flex-wrap">
            <span>
              <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">↑↓</kbd> navigate
            </span>
            <span>
              <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">1-9</kbd> jump
            </span>
            <span>
              <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">↵</kbd> open
            </span>
            <span>
              <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">L</kbd> linear
            </span>
            <span>
              <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">⌘B</kbd> branch
            </span>
            <span>
              <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">⌘⇧C</kbd> path
            </span>
            <span>
              <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">⌘D</kbd> delete
            </span>
          </div>
        </div>
      )}

      {/* Toast notification */}
      {toast && (
        <div className="absolute bottom-16 left-1/2 -translate-x-1/2 px-4 py-2 bg-bg-active border border-border rounded-lg text-xs text-text-primary shadow-xl animate-fade-in select-text">
          {toast}
        </div>
      )}

      {repo && (
        <NewWorktreeModal
          open={showNew}
          onClose={() => setShowNew(false)}
          repo={repo}
          onCreated={onWorktreeCreated}
          editorApp={editorApp}
        />
      )}
    </div>
  );
}
