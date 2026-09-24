import { useCallback, useEffect, useState } from "react";
import {
  AgentId,
  AgentViews,
  DEFAULT_AGENT_VIEWS,
  EditorApp,
  Task,
  TaskSurface,
  Workspace,
} from "../types";
import { openEditorForWorktree } from "../services/openEditor";
import { agentSurface, taskSurfaceFor, isEmbedded } from "../embedded/taskSurface";

export interface OpenTaskOptions {
  /** Open this agent in its remembered view instead of the editor choice ("Open in Codex"). */
  agent?: AgentId;
  onMessage?: (message: string) => void;
  onError?: (message: string) => void;
}

export interface OpenedTask {
  taskId: string;
  surface: TaskSurface;
}

interface UseOpenTaskArgs {
  editorApp: EditorApp;
  agentViews?: AgentViews;
  workspaces: Workspace[];
  tasks: Task[];
  recordTaskVisit: (task: Task) => void;
  showTask: (task: Task) => void;
}

/**
 * Single routing point for "open this task": external editors launch a process, embedded
 * surfaces render inside the app as the `openedTask`.
 */
export function useOpenTask({
  editorApp,
  agentViews = DEFAULT_AGENT_VIEWS,
  workspaces,
  tasks,
  recordTaskVisit,
  showTask,
}: UseOpenTaskArgs) {
  const [openedTask, setOpenedTask] = useState<OpenedTask | null>(null);

  useEffect(() => {
    if (openedTask && !tasks.some((t) => t.id === openedTask.taskId)) setOpenedTask(null);
  }, [tasks, openedTask]);

  /** Resolves to whether the task was opened (embedded, or the external editor launched). */
  const openTask = useCallback(
    async (task: Task, options: OpenTaskOptions = {}): Promise<boolean> => {
      showTask(task);
      recordTaskVisit(task);
      const surface = options.agent
        ? agentSurface(options.agent, agentViews)
        : taskSurfaceFor(editorApp, agentViews);
      if (isEmbedded(surface)) {
        setOpenedTask({ taskId: task.id, surface });
        return true;
      }
      const workspace = workspaces.find((w) => w.id === task.workspaceId);
      const result = await openEditorForWorktree(
        editorApp,
        task.members.map((m) => m.path),
        task.branchName,
        workspace?.name,
        { onMessage: options.onMessage, onError: options.onError }
      );
      return result !== null;
    },
    [editorApp, agentViews, workspaces, recordTaskVisit, showTask]
  );

  /** Open an embedded surface directly, e.g. the chat an agent notification came from. */
  const openTaskSurface = useCallback(
    (task: Task, surface: TaskSurface) => {
      showTask(task);
      recordTaskVisit(task);
      setOpenedTask({ taskId: task.id, surface });
    },
    [recordTaskVisit, showTask]
  );

  const closeTask = useCallback(() => setOpenedTask(null), []);
  const restoreTask = useCallback(
    (task: Task) => {
      showTask(task);
      const surface = taskSurfaceFor(editorApp, agentViews);
      setOpenedTask(isEmbedded(surface) ? { taskId: task.id, surface } : null);
    },
    [editorApp, agentViews, showTask]
  );

  const switchSurface = useCallback(
    (taskId: string, surface: TaskSurface) =>
      setOpenedTask((current) => (current?.taskId === taskId ? { taskId, surface } : current)),
    []
  );

  return { openedTask, openTask, openTaskSurface, closeTask, restoreTask, switchSurface };
}
