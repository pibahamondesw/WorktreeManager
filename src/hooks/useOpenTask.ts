import { useCallback, useEffect, useRef, useState } from "react";
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
import { transitionTaskView } from "../services/taskTransition";
import { SessionRegistry } from "../embedded/sessionRegistry";
import { sessionRegistry } from "../embedded/sessionTransports";
import {
  landingSurface,
  planTaskOpen,
  surfaceLabel,
  SurfaceSwitchChoice,
} from "../embedded/surfaceSwitch";

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

interface SurfaceSwitchQuestion {
  taskId: string;
  current: TaskSurface;
  requested: TaskSurface;
  closing: TaskSurface[];
  error?: string;
}

export interface SurfaceSwitchPrompt extends SurfaceSwitchQuestion {
  phase: "asking" | "closing";
}

/** The pending surface-switch question: asked as a promise, answered from the dialog. */
function useSurfaceSwitchPrompt() {
  const [prompt, setPrompt] = useState<SurfaceSwitchPrompt | null>(null);
  const answerRef = useRef<((choice: SurfaceSwitchChoice) => void) | null>(null);

  const ask = useCallback(
    (question: SurfaceSwitchQuestion) =>
      new Promise<SurfaceSwitchChoice>((resolve) => {
        answerRef.current = resolve;
        setPrompt({ ...question, phase: "asking" });
      }),
    []
  );

  const answer = useCallback((choice: SurfaceSwitchChoice) => {
    const resolve = answerRef.current;
    if (!resolve) return;
    answerRef.current = null;
    setPrompt((current) =>
      choice === "replace" && current ? { ...current, phase: "closing", error: undefined } : null
    );
    resolve(choice);
  }, []);

  const dismiss = useCallback(() => {
    const resolve = answerRef.current;
    answerRef.current = null;
    setPrompt(null);
    resolve?.("cancel");
  }, []);

  return { prompt, ask, answer, dismiss };
}

type SurfaceLifecycle = Pick<SessionRegistry, "listLive" | "stop">;

interface UseOpenTaskArgs {
  editorApp: EditorApp;
  agentViews?: AgentViews;
  workspaces: Workspace[];
  tasks: Task[];
  recordTaskVisit: (task: Task) => void;
  showTask: (task: Task) => void;
  surfaces?: SurfaceLifecycle;
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
  surfaces = sessionRegistry,
}: UseOpenTaskArgs) {
  const [openedTask, setOpenedTask] = useState<OpenedTask | null>(null);
  const surfaceSwitch = useSurfaceSwitchPrompt();
  const { ask: askSurfaceSwitch, dismiss: dismissSurfaceSwitch } = surfaceSwitch;
  const latestRequest = useRef(0);

  /** Starts a new open/close request; any older one in flight, or its prompt, is cancelled. */
  const supersede = useCallback(() => {
    dismissSurfaceSwitch();
    return ++latestRequest.current;
  }, [dismissSurfaceSwitch]);

  useEffect(() => {
    if (openedTask && !tasks.some((t) => t.id === openedTask.taskId)) setOpenedTask(null);
  }, [tasks, openedTask]);

  const promptTaskId = surfaceSwitch.prompt?.taskId;
  useEffect(() => {
    if (promptTaskId && !tasks.some((t) => t.id === promptTaskId)) supersede();
  }, [tasks, promptTaskId, supersede]);

  const liveSurfaces = useCallback(
    (taskId: string) => surfaces.listLive(taskId).catch((): TaskSurface[] => []),
    [surfaces]
  );

  /** Closes in order; on failure, returns the error and the surfaces still open. */
  const closeSurfaces = useCallback(
    async (taskId: string, closing: TaskSurface[]) => {
      for (const [index, surface] of closing.entries()) {
        try {
          await surfaces.stop(taskId, surface);
        } catch (error) {
          const message = `Could not close ${surfaceLabel(surface)}: ${String(error)}`;
          return { error: message, remaining: closing.slice(index) };
        }
      }
      return null;
    },
    [surfaces]
  );

  /** The surface to show, or null when the user cancels or a newer request wins. */
  const settleSurface = useCallback(
    async (task: Task, requested: TaskSurface, request: number) => {
      const plan = planTaskOpen(await liveSurfaces(task.id), requested);
      if (request !== latestRequest.current) return null;
      if (plan.kind === "open") return plan.surface;
      let question: SurfaceSwitchQuestion = {
        taskId: task.id,
        current: plan.current,
        requested,
        closing: plan.closing,
      };
      let choice = await askSurfaceSwitch(question);
      while (choice === "replace") {
        const failure = await closeSurfaces(task.id, question.closing);
        if (request !== latestRequest.current) return null;
        if (!failure) {
          dismissSurfaceSwitch();
          return requested;
        }
        question = { ...question, closing: failure.remaining, error: failure.error };
        choice = await askSurfaceSwitch(question);
      }
      return choice === "keep" ? plan.current : null;
    },
    [liveSurfaces, askSurfaceSwitch, closeSurfaces, dismissSurfaceSwitch]
  );

  const showSurface = useCallback(
    (task: Task, surface: TaskSurface) =>
      transitionTaskView(task.id, () => {
        showTask(task);
        recordTaskVisit(task);
        setOpenedTask({ taskId: task.id, surface });
      }),
    [recordTaskVisit, showTask]
  );

  /** Resolves to whether the task was opened (embedded, or the external editor launched). */
  const openTask = useCallback(
    async (task: Task, options: OpenTaskOptions = {}): Promise<boolean> => {
      const request = supersede();
      const requested = options.agent
        ? agentSurface(options.agent, agentViews)
        : taskSurfaceFor(editorApp, agentViews, task.agent);
      if (isEmbedded(requested)) {
        const surface = await settleSurface(task, requested, request);
        if (!surface) return false;
        showSurface(task, surface);
        return true;
      }
      showTask(task);
      recordTaskVisit(task);
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
    [
      editorApp,
      agentViews,
      workspaces,
      recordTaskVisit,
      showTask,
      supersede,
      settleSurface,
      showSurface,
    ]
  );

  /** Open the surface an agent notification came from; it asks only if another one replaced it. */
  const openTaskSurface = useCallback(
    (task: Task, surface: TaskSurface) => {
      const request = supersede();
      void settleSurface(task, surface, request).then((settled) => {
        if (settled) showSurface(task, settled);
      });
    },
    [supersede, settleSurface, showSurface]
  );

  const closeTask = useCallback(
    ({ animate = true }: { animate?: boolean } = {}) => {
      supersede();
      if (!openedTask) return;
      if (animate) transitionTaskView(openedTask.taskId, () => setOpenedTask(null));
      else setOpenedTask(null);
    },
    [openedTask, supersede]
  );

  /** History navigation never asks or closes anything: it lands on the live surface if any. */
  const restoreTask = useCallback(
    (task: Task) => {
      const request = supersede();
      const requested = taskSurfaceFor(editorApp, agentViews, task.agent);
      void liveSurfaces(task.id).then((live) => {
        if (request !== latestRequest.current) return;
        const surface = landingSurface(live, requested);
        transitionTaskView(task.id, () => {
          showTask(task);
          setOpenedTask(isEmbedded(surface) ? { taskId: task.id, surface } : null);
        });
      });
    },
    [editorApp, agentViews, showTask, supersede, liveSurfaces]
  );

  const switchSurface = useCallback(
    (taskId: string, surface: TaskSurface) =>
      setOpenedTask((current) => (current?.taskId === taskId ? { taskId, surface } : current)),
    []
  );

  return {
    openedTask,
    openTask,
    openTaskSurface,
    closeTask,
    restoreTask,
    switchSurface,
    surfaceSwitch: surfaceSwitch.prompt,
    resolveSurfaceSwitch: surfaceSwitch.answer,
  };
}
