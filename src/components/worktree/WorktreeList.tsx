import { useCallback, useState, useEffect, useMemo, useRef } from "react";
import { LinearProvider } from "../../contexts/LinearContext";
import { LinearService } from "../../services/linear";
import { useEphemeralToast } from "../../hooks/useEphemeralToast";
import { useRevealRing } from "../../hooks/useRevealRing";
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
import { DepartingCard } from "./DepartingCard";
import { LinkIssueModal } from "./LinkIssueModal";
import { NewWorktreeModal } from "./NewWorktreeModal";
import { Task, TaskSurface, VaultConfig, Workspace, EditorApp, GitStatus } from "../../types";
import { TaskView } from "../task/TaskView";
import { OpenedTask, OpenTaskOptions } from "../../hooks/useOpenTask";
import {
  CreateTaskInput,
  DeleteOptions,
  OperationResult,
  TaskReady,
} from "../../services/operations";
import { TerminalStatus } from "../../services/terminal";
import { AgentActivities } from "../../services/agentActivity";

interface WorktreeListProps {
  tasks: Task[];
  agentSessions?: Record<string, TerminalStatus>;
  agentActivities?: AgentActivities;
  workspace: Workspace | undefined;
  vault: VaultConfig;
  onTaskCreated: (
    input: CreateTaskInput,
    progress?: (message: string) => void,
    onReady?: TaskReady
  ) => Promise<OperationResult<Task>>;
  onTaskPinned: (id: string, pinned: boolean) => Promise<void>;
  onPinnedTasksReordered: (id: string, targetId: string) => Promise<void>;
  onTaskIssueLinked: (taskId: string, issue: string) => Promise<Task>;
  onTaskDeleted: (
    taskId: string,
    options: DeleteOptions
  ) => Promise<OperationResult<{ id: string }>>;
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
  openedTask: OpenedTask | null;
  onOpenTask: (task: Task, options?: OpenTaskOptions) => Promise<boolean>;
  onCloseTask: () => void;
  onSwitchSurface: (taskId: string, surface: TaskSurface) => void;
  canGoBack: boolean;
  canGoForward: boolean;
  onGoBack: () => void;
  onGoForward: () => void;
  /** Palette asking this list to open the new-task modal. */
  requestNewTask?: boolean;
  onRequestNewTaskHandled?: () => void;
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

const CARD_GAP = 12;

interface DepartingTask {
  task: Task;
  index: number;
}

function departedTasks(
  previous: Task[],
  current: Task[],
  deletingIds: ReadonlySet<string>
): DepartingTask[] {
  const remaining = new Set(current.map((task) => task.id));
  return previous.flatMap((task, index) =>
    deletingIds.has(task.id) && !remaining.has(task.id) ? [{ task, index }] : []
  );
}

function withDeparting(tasks: Task[], departing: DepartingTask[]) {
  const rows = tasks.map((task) => ({ task, departing: false }));
  for (const item of [...departing].sort((a, b) => a.index - b.index))
    rows.splice(Math.min(item.index, rows.length), 0, { task: item.task, departing: true });
  return rows;
}

const SELECTED_CARD_SCROLL_ATTEMPTS = 60;

export function WorktreeList({
  tasks,
  agentSessions = {},
  agentActivities = {},
  workspace,
  vault,
  onTaskCreated,
  onTaskDeleted,
  onTaskIssueLinked,
  onTaskPinned,
  onPinnedTasksReordered,
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
  openedTask,
  onOpenTask,
  onCloseTask,
  onSwitchSurface,
  canGoBack,
  canGoForward,
  onGoBack,
  onGoForward,
  requestNewTask,
  onRequestNewTaskHandled,
}: WorktreeListProps) {
  const [linkTaskId, setLinkTaskId] = useState<string | null>(null);
  const linkTask = tasks.find((task) => task.id === linkTaskId && !task.linearIssueId);
  const [showNew, setShowNew] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const [deleteRequested, setDeleteRequested] = useState(false);
  const [revealNonce, setRevealNonce] = useState(0);
  const [deletingIds, setDeletingIds] = useState<ReadonlySet<string>>(new Set());
  const [departing, setDeparting] = useState<DepartingTask[]>([]);
  const finishDeparture = useCallback(
    (taskId: string) =>
      setDeparting((current) => current.filter((item) => item.task.id !== taskId)),
    []
  );
  const [previousTasks, setPreviousTasks] = useState(tasks);
  if (previousTasks !== tasks) {
    setPreviousTasks(tasks);
    const deleted = departedTasks(previousTasks, tasks, deletingIds);
    if (deleted.length) setDeparting((current) => [...current, ...deleted]);
  }
  const { toast, showToast } = useEphemeralToast();
  const { ring, ringTask, clearRing } = useRevealRing();
  const draggedTaskId = useRef<string | null>(null);
  const [dragOverId, setDragOverId] = useState<string | null>(null);
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
    if (!requestNewTask) return;
    setShowNew(true);
    onRequestNewTaskHandled?.();
  }, [requestNewTask, onRequestNewTaskHandled]);

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
    if (!openedTask) ringTask(revealTaskId);
    onRevealHandled();
  }, [revealTaskId, tasks, onRevealHandled, openedTask, ringTask]);

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

  const selectableTask =
    selectedIndex >= 0 && selectedIndex < tasks.length ? tasks[selectedIndex] : null;
  const selectedTask =
    selectableTask && !deletingIds.has(selectableTask.id) ? selectableTask : null;

  const deleteTask: typeof onTaskDeleted = async (id, options) => {
    setDeletingIds((ids) => new Set(ids).add(id));
    try {
      return await onTaskDeleted(id, options);
    } catch (error) {
      showToast(error instanceof Error ? error.message : String(error));
      throw error;
    } finally {
      setDeletingIds((ids) => {
        const next = new Set(ids);
        next.delete(id);
        return next;
      });
    }
  };

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
    searchOpen: searchOpen || !!linkTask,
    setShowNew,
    setSelectedIndex,
    jumpToIndex: (index) => {
      setSelectedIndex(index);
      ringTask(tasks[index].id);
    },
    setDeleteRequested,
    handleRefresh,
    showToast,
    onOpenTask,
    taskOpen: openTask !== null,
  });

  if (!workspace) return <WorktreeNoRepoPlaceholder />;

  const renderCard = (task: Task, i: number) => (
    <WorktreeCard
      onTogglePin={() => {
        void onTaskPinned(task.id, task.pinOrder === undefined)
          .then(() => setSelectedIndex(-1))
          .catch((error) => showToast(String(error)));
      }}
      task={task}
      workspace={workspace}
      vault={vault}
      onDelete={deleteTask}
      onLinkIssue={() => setLinkTaskId(task.id)}
      linearInfo={task.linearIssueId ? linearInfo[task.linearIssueId] : undefined}
      gitStatus={aggregateTaskStatus(task, gitStatuses)}
      selected={i >= 0 && i === selectedIndex}
      index={i}
      sessionStatus={agentSessions[task.id]}
      agentActivity={agentActivities[task.id]}
      onOpenError={showToast}
      onToast={showToast}
      onOpen={() => void onOpenTask(task, { onMessage: showToast, onError: showToast })}
      repoSlugs={repoSlugs}
      requestDelete={i >= 0 && i === selectedIndex && deleteRequested}
      onRequestDeleteHandled={() => setDeleteRequested(false)}
    />
  );

  const taskOpen = openTask !== null && openedTask !== null;

  return (
    <LinearProvider apiKey={workspace.linearApiKey ?? null}>
      <div className="flex-1 flex flex-col min-h-0 min-w-0 relative">
        {openTask && openedTask && (
          <TaskView
            key={openTask.id}
            task={openTask}
            surface={openedTask.surface}
            linearInfo={openTask.linearIssueId ? linearInfo[openTask.linearIssueId] : undefined}
            gitStatus={aggregateTaskStatus(openTask, gitStatuses)}
            sidebarCollapsed={sidebarCollapsed}
            onExpandSidebar={onExpandSidebar}
            onBack={handleCloseTask}
            onSwitchSurface={(surface) => onSwitchSurface(openTask.id, surface)}
            agentActivity={agentActivities[openTask.id]}
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
              <div key="skeleton" className="grid gap-3">
                {Array.from({ length: Math.max(tasks.length, 3) }).map((_, i) => (
                  <WorktreeCardSkeleton key={i} index={i} />
                ))}
              </div>
            ) : tasks.length === 0 ? (
              <WorktreeEmptyWorktrees onCreateFirst={() => setShowNew(true)} />
            ) : (
              <div key="tasks" className="grid gap-3 motion-rise">
                {withDeparting(tasks, departing).map(({ task, departing: leaving }) =>
                  leaving ? (
                    <DepartingCard
                      key={task.id}
                      gap={CARD_GAP}
                      taskId={task.id}
                      onDeparted={finishDeparture}
                    >
                      {renderCard(task, -1)}
                    </DepartingCard>
                  ) : (
                    <div
                      key={task.id}
                      draggable={task.pinOrder !== undefined}
                      onDragStart={(event) => {
                        draggedTaskId.current = task.id;
                        event.dataTransfer.effectAllowed = "move";
                        event.dataTransfer.setData("text/plain", task.id);
                      }}
                      onDragOver={(event) => {
                        if (!draggedTaskId.current || task.pinOrder === undefined) return;
                        event.preventDefault();
                        event.dataTransfer.dropEffect = "move";
                        setDragOverId(task.id);
                      }}
                      onDrop={(event) => {
                        event.preventDefault();
                        const source = draggedTaskId.current;
                        draggedTaskId.current = null;
                        setDragOverId(null);
                        if (!source || source === task.id || task.pinOrder === undefined) return;
                        void onPinnedTasksReordered(source, task.id)
                          .then(() => setSelectedIndex(-1))
                          .catch((error) => showToast(String(error)));
                      }}
                      onDragEnd={() => {
                        draggedTaskId.current = null;
                        setDragOverId(null);
                      }}
                      inert={deletingIds.has(task.id)}
                      className={`relative transition-opacity ${
                        deletingIds.has(task.id) ? "opacity-50 saturate-50" : ""
                      } ${
                        dragOverId === task.id && draggedTaskId.current !== task.id
                          ? "rounded-xl outline outline-2 outline-accent"
                          : ""
                      }`}
                    >
                      {ring?.taskId === task.id && (
                        <span
                          key={ring.nonce}
                          data-testid="reveal-ring"
                          aria-hidden="true"
                          className="motion-reveal-ring rounded-xl z-10"
                          onAnimationEnd={clearRing}
                        />
                      )}
                      {renderCard(task, tasks.indexOf(task))}
                    </div>
                  )
                )}
              </div>
            )}
          </div>

          {tasks.length > 0 && <WorktreeListKeyboardHints showNotes={vault.enabled} />}
        </div>

        {toast && <WorktreeListToast message={toast} />}

        {linkTask && (
          <LinkIssueModal
            key={linkTask.id}
            task={linkTask}
            configured={!!workspace.linearApiKey}
            onLink={onTaskIssueLinked}
            onClose={() => setLinkTaskId(null)}
          />
        )}
        <NewWorktreeModal
          open={showNew}
          onClose={() => setShowNew(false)}
          workspace={workspace}
          onCreated={onTaskCreated}
          onOpenTask={onOpenTask}
          editorApp={editorApp}
          onOpenHint={showToast}
        />
      </div>
    </LinearProvider>
  );
}
