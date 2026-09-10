import { useCallback, useEffect, useState } from "react";
import { EditorApp, Task, TaskSurface, Workspace } from "../types";
import { openEditorForWorktree } from "../services/openEditor";
import { taskSurfaceFor, isEmbedded } from "../embedded/taskSurface";

export interface OpenTaskOptions {
  /** Override the editor-derived surface (e.g. the "Open in Claude Code" menu action). */
  surface?: TaskSurface;
  onMessage?: (message: string) => void;
  onError?: (message: string) => void;
}

export interface OpenedTask {
  taskId: string;
  surface: TaskSurface;
}

interface UseOpenTaskArgs {
  editorApp: EditorApp;
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
      const surface = options.surface ?? taskSurfaceFor(editorApp);
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
    [editorApp, workspaces, recordTaskVisit, showTask]
  );

  const closeTask = useCallback(() => setOpenedTask(null), []);
  const restoreTask = useCallback(
    (task: Task) => {
      showTask(task);
      const surface = taskSurfaceFor(editorApp);
      setOpenedTask(isEmbedded(surface) ? { taskId: task.id, surface } : null);
    },
    [editorApp, showTask]
  );

  return { openedTask, openTask, closeTask, restoreTask };
}
