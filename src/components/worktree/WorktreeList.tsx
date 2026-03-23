import { useState, useEffect } from "react";
import { LinearProvider } from "../../contexts/LinearContext";
import { useEphemeralToast } from "../../hooks/useEphemeralToast";
import { useWorktreeListKeyboardShortcuts } from "../../hooks/useWorktreeListKeyboardShortcuts";
import { useWorktreeData } from "../../hooks/useWorktreeData";
import { WorktreeCard } from "./WorktreeCard";
import { WorktreeCardSkeleton } from "./WorktreeCardSkeleton";
import { WorktreeEmptyWorktrees, WorktreeNoRepoPlaceholder } from "./WorktreeListEmptyStates";
import { WorktreeListHeader } from "./WorktreeListHeader";
import { WorktreeListKeyboardHints } from "./WorktreeListKeyboardHints";
import { WorktreeListToast } from "./WorktreeListToast";
import { NewWorktreeModal } from "./NewWorktreeModal";
import { Worktree, Repo, EditorApp } from "../../types";

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
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const [deleteRequested, setDeleteRequested] = useState(false);
  const { toast, showToast } = useEphemeralToast();

  const { linearInfo, gitStatuses, enrichmentLoading, refreshing, handleRefresh } = useWorktreeData(worktrees, repo);

  useEffect(() => {
    setSelectedIndex(-1);
  }, [repo?.id]);

  const selectedWorktree =
    selectedIndex >= 0 && selectedIndex < worktrees.length ? worktrees[selectedIndex] : null;

  useWorktreeListKeyboardShortcuts({
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
  });

  if (!repo) return <WorktreeNoRepoPlaceholder />;

  return (
    <LinearProvider apiKey={repo.linearApiKey ?? null}>
      <div className="flex-1 flex flex-col min-h-0 relative">
        <WorktreeListHeader
          repoName={repo.name}
          worktreeCount={worktrees.length}
          editorApp={editorApp}
          onEditorChange={onEditorChange}
          onRefresh={handleRefresh}
          refreshing={refreshing}
          onNewWorktree={() => setShowNew(true)}
        />

        <div className="flex-1 overflow-y-auto p-6">
          {enrichmentLoading ? (
            <div className="grid gap-3">
              {Array.from({ length: Math.max(worktrees.length, 3) }).map((_, i) => (
                <WorktreeCardSkeleton key={i} index={i + 1} />
              ))}
            </div>
          ) : worktrees.length === 0 ? (
            <WorktreeEmptyWorktrees onCreateFirst={() => setShowNew(true)} />
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

        {worktrees.length > 0 && <WorktreeListKeyboardHints />}

        {toast && <WorktreeListToast message={toast} />}

        <NewWorktreeModal
          open={showNew}
          onClose={() => setShowNew(false)}
          repo={repo}
          onCreated={onWorktreeCreated}
          editorApp={editorApp}
        />
      </div>
    </LinearProvider>
  );
}
