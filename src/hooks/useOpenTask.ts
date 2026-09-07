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
export function useOpenTask({ editorApp, workspaces, tasks, recordTaskVisit, showTask }: UseOpenTaskArgs) {
  const [openedTask, setOpenedTask] = useState<OpenedTask | null>(null);

  useEffect(() => {
    if (openedTask && !tasks.some((t) => t.id === openedTask.taskId)) setOpenedTask(null);
  }, [tasks, openedTask]);

  const openTask = useCallback(
    async (task: Task, options: OpenTaskOptions = {}) => {
      recordTaskVisit(task);
      const surface = options.surface ?? taskSurfaceFor(editorApp);
      if (isEmbedded(surface)) {
        showTask(task);
        setOpenedTask({ taskId: task.id, surface });
        return;
      }
      const workspace = workspaces.find((w) => w.id === task.workspaceId);
      await openEditorForWorktree(
        editorApp,
        task.members.map((m) => m.path),
        task.branchName,
        workspace?.name,
        { onMessage: options.onMessage, onError: options.onError }
      );
    },
    [editorApp, workspaces, recordTaskVisit, showTask]
  );

  const closeTask = useCallback(() => setOpenedTask(null), []);

  return { openedTask, openTask, closeTask };
}
