import { useState, useEffect, useMemo } from "react";
import { LinearProvider } from "../../contexts/LinearContext";
import { LinearService } from "../../services/linear";
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
import { Task, Workspace, EditorApp, GitStatus } from "../../types";

interface WorktreeListProps {
  tasks: Task[];
  workspace: Workspace | undefined;
  onTaskCreated: (task: Task) => void;
  onTaskDeleted: (taskId: string) => void;
  editorApp: EditorApp;
  onEditorChange: (editor: EditorApp) => void;
  workspaceSwitching: boolean;
  onWorkspaceReady: () => void;
}

/** Collapse the per-member git statuses of a task into one summary for its card. */
function aggregateTaskStatus(
  task: Task,
  gitStatuses: Record<string, GitStatus>,
): GitStatus | undefined {
  const statuses = task.members
    .map((m) => gitStatuses[m.path])
    .filter((s): s is GitStatus => !!s);
  if (statuses.length === 0) return undefined;
  return {
    ahead: Math.max(...statuses.map((s) => s.ahead)),
    behind: Math.max(...statuses.map((s) => s.behind)),
    dirty: statuses.some((s) => s.dirty),
    // Oldest commit across the task's worktrees drives the staleness indicator.
    last_commit_epoch: Math.min(
      ...statuses.map((s) => s.last_commit_epoch).filter((e) => e > 0),
    ),
  };
}

export function WorktreeList({
  tasks,
  workspace,
  onTaskCreated,
  onTaskDeleted,
  editorApp,
  onEditorChange,
  workspaceSwitching,
  onWorkspaceReady,
}: WorktreeListProps) {
  const [showNew, setShowNew] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const [deleteRequested, setDeleteRequested] = useState(false);
  const { toast, showToast } = useEphemeralToast();

  const linearApiKey = workspace?.linearApiKey;
  const linearService = useMemo(
    () => (linearApiKey ? new LinearService(linearApiKey) : null),
    [linearApiKey]
  );

  const { linearInfo, gitStatuses, refreshing, handleRefresh } = useWorktreeData(
    tasks,
    workspace,
    linearService,
    onWorkspaceReady
  );

  useEffect(() => {
    setSelectedIndex(-1);
  }, [workspace?.id]);

  const selectedTask =
    selectedIndex >= 0 && selectedIndex < tasks.length ? tasks[selectedIndex] : null;

  useWorktreeListKeyboardShortcuts({
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
  });

  if (!workspace) return <WorktreeNoRepoPlaceholder />;

  return (
    <LinearProvider apiKey={workspace.linearApiKey ?? null}>
      <div className="flex-1 flex flex-col min-h-0 min-w-0 relative">
        <WorktreeListHeader
          workspaceName={workspace.name}
          taskCount={tasks.length}
          repoCount={workspace.repos.length}
          editorApp={editorApp}
          onEditorChange={onEditorChange}
          onRefresh={handleRefresh}
          refreshing={refreshing}
          onNewTask={() => setShowNew(true)}
        />

        <div className="flex-1 overflow-y-auto p-6">
          {workspaceSwitching ? (
            <div className="grid gap-3">
              {Array.from({ length: Math.max(tasks.length, 3) }).map((_, i) => (
                <WorktreeCardSkeleton key={i} index={i + 1} />
              ))}
            </div>
          ) : tasks.length === 0 ? (
            <WorktreeEmptyWorktrees onCreateFirst={() => setShowNew(true)} />
          ) : (
            <div className="grid gap-3">
              {tasks.map((task, i) => (
                <WorktreeCard
                  key={task.id}
                  task={task}
                  workspace={workspace}
                  onDelete={onTaskDeleted}
                  linearInfo={task.linearIssueId ? linearInfo[task.linearIssueId] : undefined}
                  gitStatus={aggregateTaskStatus(task, gitStatuses)}
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

        {tasks.length > 0 && <WorktreeListKeyboardHints />}

        {toast && <WorktreeListToast message={toast} />}

        <NewWorktreeModal
          open={showNew}
          onClose={() => setShowNew(false)}
          workspace={workspace}
          onCreated={onTaskCreated}
          editorApp={editorApp}
          onOpenHint={showToast}
        />
      </div>
    </LinearProvider>
  );
}
