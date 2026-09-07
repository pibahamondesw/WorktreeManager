import { useState, useEffect, useMemo, useRef } from "react";
import { LinearProvider } from "../../contexts/LinearContext";
import { LinearService } from "../../services/linear";
import { useEphemeralToast } from "../../hooks/useEphemeralToast";
import { useWorktreeListKeyboardShortcuts } from "../../hooks/useWorktreeListKeyboardShortcuts";
import { useWorktreeData } from "../../hooks/useWorktreeData";
import { useRepoSlugs } from "../../hooks/useRepoSlugs";
import { WorktreeCard } from "./WorktreeCard";
import { cardScrollDelta } from "./cardScroll";
import { WorktreeCardSkeleton } from "./WorktreeCardSkeleton";
import { WorktreeEmptyWorktrees, WorktreeNoRepoPlaceholder } from "./WorktreeListEmptyStates";
import { WorktreeListHeader } from "./WorktreeListHeader";
import { WorktreeListKeyboardHints } from "./WorktreeListKeyboardHints";
import { WorktreeListToast } from "./WorktreeListToast";
import { NewWorktreeModal } from "./NewWorktreeModal";
import { Task, VaultConfig, Workspace, EditorApp, GitStatus } from "../../types";
import { TaskView } from "../task/TaskView";
import { OpenedTask, OpenTaskOptions } from "../../hooks/useOpenTask";

interface WorktreeListProps {
  tasks: Task[];
  workspace: Workspace | undefined;
  vault: VaultConfig;
  onTaskCreated: (task: Task) => void;
  onTaskDeleted: (taskId: string) => void;
  editorApp: EditorApp;
  onEditorChange: (editor: EditorApp) => void;
  workspaceSwitching: boolean;
  onWorkspaceReady: (workspaceId?: string) => void;
  onOpenSearch: () => void;
  sidebarCollapsed: boolean;
  onExpandSidebar: () => void;
  searchOpen: boolean;
  /** Task picked in the quick search: select it once this workspace's tasks are in. */
  revealTaskId: string | null;
  onRevealHandled: () => void;
  /** A task was opened in the editor (card, Enter, or just created): record the visit. */
  onTaskOpened: (task: Task) => void;
  openedTask: OpenedTask | null;
  onOpenTask: (task: Task, options?: OpenTaskOptions) => Promise<void>;
  onCloseTask: () => void;
  canGoBack: boolean;
  canGoForward: boolean;
  onGoBack: () => void;
  onGoForward: () => void;
}

/** Collapse the per-member git statuses of a task into one summary for its card. */
function aggregateTaskStatus(
  task: Task,
  gitStatuses: Record<string, GitStatus>
): GitStatus | undefined {
  const statuses = task.members.map((m) => gitStatuses[m.path]).filter((s): s is GitStatus => !!s);
  if (statuses.length === 0) return undefined;
  return {
    ahead: Math.max(...statuses.map((s) => s.ahead)),
    behind: Math.max(...statuses.map((s) => s.behind)),
    dirty: statuses.some((s) => s.dirty),
    // Oldest commit across the task's worktrees drives the staleness indicator.
    last_commit_epoch: Math.min(...statuses.map((s) => s.last_commit_epoch).filter((e) => e > 0)),
  };
}

const SELECTED_CARD_SCROLL_ATTEMPTS = 60;

export function WorktreeList({
  tasks,
  workspace,
  vault,
  onTaskCreated,
  onTaskDeleted,
  editorApp,
  onEditorChange,
  workspaceSwitching,
  onWorkspaceReady,
  onOpenSearch,
  sidebarCollapsed,
  onExpandSidebar,
  searchOpen,
  revealTaskId,
  onRevealHandled,
  onTaskOpened,
  openedTask,
  onOpenTask,
  onCloseTask,
  canGoBack,
  canGoForward,
  onGoBack,
  onGoForward,
}: WorktreeListProps) {
  const [showNew, setShowNew] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const [deleteRequested, setDeleteRequested] = useState(false);
  const [revealNonce, setRevealNonce] = useState(0);
  const { toast, showToast } = useEphemeralToast();
  const listRef = useRef<HTMLDivElement>(null);

  const linearApiKey = workspace?.linearApiKey;
  const linearService = useMemo(
    () => (linearApiKey ? new LinearService(linearApiKey) : null),
    [linearApiKey]
  );

  const repoSlugs = useRepoSlugs(workspace);
  const { linearInfo, gitStatuses, refreshing, handleRefresh } = useWorktreeData(
    tasks,
    workspace,
    linearService,
    onWorkspaceReady
  );

  useEffect(() => {
    setSelectedIndex(-1);
  }, [workspace?.id]);

  // Declared after the reset above so a reveal wins when a workspace switch lands in the
  // same commit: the task may only appear in `tasks` after that switch.
  useEffect(() => {
    if (!revealTaskId) return;
    const index = tasks.findIndex((t) => t.id === revealTaskId);
    if (index === -1) return;
    setSelectedIndex(index);
    setRevealNonce((n) => n + 1);
    onRevealHandled();
  }, [revealTaskId, tasks, onRevealHandled]);

  // The card may not be mounted yet when a reveal lands mid workspace switch (skeletons are
  // rendered instead). Retries on a timer rather than rAF, which is throttled to nothing while
  // the window sits behind a just-opened editor.
  useEffect(() => {
    if (selectedIndex < 0) return;
    let attempts = 0;
    const focusSelectedCard = () => {
      const container = listRef.current;
      const card = container?.querySelector<HTMLElement>('[data-selected="true"]');
      if (!container || !card) {
        if (attempts++ < SELECTED_CARD_SCROLL_ATTEMPTS) timer = setTimeout(focusSelectedCard, 32);
        return;
      }
      container.scrollTop += cardScrollDelta(
        container.getBoundingClientRect(),
        card.getBoundingClientRect()
      );
      card.focus({ preventScroll: true });
    };
    let timer = setTimeout(focusSelectedCard, 0);
    return () => clearTimeout(timer);
  }, [selectedIndex, revealNonce, tasks, workspaceSwitching]);

  const selectedTask =
    selectedIndex >= 0 && selectedIndex < tasks.length ? tasks[selectedIndex] : null;

  const openTask = openedTask ? (tasks.find((t) => t.id === openedTask.taskId) ?? null) : null;

  const handleCloseTask = () => {
    if (openTask) {
      setSelectedIndex(tasks.indexOf(openTask));
      setRevealNonce((n) => n + 1);
    }
    onCloseTask();
  };

  useWorktreeListKeyboardShortcuts({
    workspace,
    vault,
    tasks,
    selectedTask,
    showNew,
    searchOpen,
    setShowNew,
    setSelectedIndex,
    setDeleteRequested,
    handleRefresh,
    showToast,
    onOpenTask,
    taskOpen: openTask !== null,
  });

  if (!workspace) return <WorktreeNoRepoPlaceholder />;

  const taskOpen = openTask !== null && openedTask !== null;

  return (
    <LinearProvider apiKey={workspace.linearApiKey ?? null}>
      <div className="flex-1 flex flex-col min-h-0 min-w-0 relative">
        {openTask && openedTask && (
          <TaskView
            task={openTask}
            surface={openedTask.surface}
            sidebarCollapsed={sidebarCollapsed}
            onExpandSidebar={onExpandSidebar}
            onBack={handleCloseTask}
          />
        )}
        {/* The grid stays mounted while a task is open so returning is instant (no re-probing of
            editors, no card remounts). */}
        <div className="contents" hidden={taskOpen}>
        <WorktreeListHeader
          workspaceName={workspace.name}
          taskCount={tasks.length}
          repoCount={workspace.repos.length}
          editorApp={editorApp}
          onEditorChange={onEditorChange}
          onRefresh={handleRefresh}
          refreshing={refreshing}
          onNewTask={() => setShowNew(true)}
          onOpenSearch={onOpenSearch}
          sidebarCollapsed={sidebarCollapsed}
          onExpandSidebar={onExpandSidebar}
          canGoBack={canGoBack}
          canGoForward={canGoForward}
          onGoBack={onGoBack}
          onGoForward={onGoForward}
        />

        <div ref={listRef} className="flex-1 overflow-y-auto p-6">
          {workspaceSwitching ? (
            <div className="grid gap-3">
              {Array.from({ length: Math.max(tasks.length, 3) }).map((_, i) => (
                <WorktreeCardSkeleton key={i} index={i} />
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
                  vault={vault}
                  onDelete={onTaskDeleted}
                  linearInfo={task.linearIssueId ? linearInfo[task.linearIssueId] : undefined}
                  gitStatus={aggregateTaskStatus(task, gitStatuses)}
                  selected={i === selectedIndex}
                  index={i}
                  onOpenError={showToast}
                  onToast={showToast}
                  onOpen={() => void onOpenTask(task, { onMessage: showToast, onError: showToast })}
                  repoSlugs={repoSlugs}
                  requestDelete={i === selectedIndex && deleteRequested}
                  onRequestDeleteHandled={() => setDeleteRequested(false)}
                />
              ))}
            </div>
          )}
        </div>

        {tasks.length > 0 && <WorktreeListKeyboardHints showNotes={vault.enabled} />}
        </div>

        {toast && <WorktreeListToast message={toast} />}

        <NewWorktreeModal
          open={showNew}
          onClose={() => setShowNew(false)}
          workspace={workspace}
          vault={vault}
          onCreated={onTaskCreated}
          onTaskOpened={onTaskOpened}
          editorApp={editorApp}
          onOpenHint={showToast}
        />
      </div>
    </LinearProvider>
  );
}
