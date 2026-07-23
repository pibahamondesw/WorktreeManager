import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { loadState, loadEditorApp, loadThemeId, loadCustomColors, persist } from "../services/store";
import { AppState, DEFAULT_STATE, EditorApp, Task, Workspace } from "../types";
import { applyTheme, themes, CUSTOM_THEME_ID } from "../themes";

export function useStore() {
  const [state, setState] = useState<AppState>(DEFAULT_STATE);
  const [loading, setLoading] = useState(true);
  const [editorApp, setEditorAppState] = useState<EditorApp>("cursor");
  const [themeId, setThemeIdState] = useState("default");
  const [customColors, setCustomColors] = useState<Record<string, string> | null>(null);
  const [workspaceSwitching, setWorkspaceSwitching] = useState(true);
  const [persistError, setPersistError] = useState<string | null>(null);
  // Workspaces whose data finished loading at least once this session: switching
  // back to them shows the in-memory data immediately instead of the skeleton.
  const loadedWorkspaceIdsRef = useRef(new Set<string>());

  useEffect(() => {
    Promise.all([loadState(), loadEditorApp(), loadThemeId(), loadCustomColors()]).then(
      ([s, editor, theme, custom]) => {
        setState(s);
        setEditorAppState(editor);
        setThemeIdState(theme);
        setCustomColors(custom);
        applyTheme(theme, theme === CUSTOM_THEME_ID ? (custom ?? undefined) : undefined);
        setLoading(false);

        const basePaths = [
          ...new Set(s.workspaces.flatMap((w) => w.repos.map((r) => r.worktreeBasePath))),
        ];
        void invoke("cleanup_claude_json_stale", { basePaths }).catch(() => {});
      },
    );
  }, []);

  const dismissPersistError = useCallback(() => setPersistError(null), []);

  const updateSetup = useCallback(async (setup: AppState["setup"]) => {
    let snapshot: AppState;
    setState((prev) => {
      snapshot = prev;
      return { ...prev, setup };
    });
    try {
      await persist([["setup", setup]]);
    } catch {
      setState(snapshot!);
      setPersistError("Failed to save setup changes");
    }
  }, []);

  const addWorkspace = useCallback(async (workspace: Workspace) => {
    let snapshot: AppState;
    let newWorkspaces: Workspace[];
    setState((prev) => {
      snapshot = prev;
      newWorkspaces = [...prev.workspaces, workspace];
      return { ...prev, workspaces: newWorkspaces, selectedWorkspaceId: workspace.id };
    });
    try {
      await persist([
        ["workspaces", newWorkspaces!],
        ["selectedWorkspaceId", workspace.id],
      ]);
    } catch {
      setState(snapshot!);
      setPersistError("Failed to save workspace");
    }
  }, []);

  const updateWorkspace = useCallback(
    async (
      workspaceId: string,
      updates: Partial<Pick<Workspace, "name" | "linearApiKey" | "repos">>,
    ) => {
      let snapshot: AppState;
      let newWorkspaces: Workspace[];
      setState((prev) => {
        snapshot = prev;
        newWorkspaces = prev.workspaces.map((w) =>
          w.id === workspaceId ? { ...w, ...updates } : w,
        );
        return { ...prev, workspaces: newWorkspaces };
      });
      try {
        await persist([["workspaces", newWorkspaces!]]);
      } catch {
        setState(snapshot!);
        setPersistError("Failed to save workspace changes");
      }
    },
    [],
  );

  const removeWorkspace = useCallback(async (workspaceId: string) => {
    loadedWorkspaceIdsRef.current.delete(workspaceId);
    let snapshot: AppState;
    let newWorkspaces: Workspace[];
    let newTasks: Task[];
    let newSelectedId: string | null;
    setState((prev) => {
      snapshot = prev;
      newWorkspaces = prev.workspaces.filter((w) => w.id !== workspaceId);
      newTasks = prev.tasks.filter((t) => t.workspaceId !== workspaceId);
      newSelectedId =
        prev.selectedWorkspaceId === workspaceId
          ? (newWorkspaces[0]?.id ?? null)
          : prev.selectedWorkspaceId;
      return {
        ...prev,
        workspaces: newWorkspaces,
        tasks: newTasks,
        selectedWorkspaceId: newSelectedId,
      };
    });
    try {
      await persist([
        ["workspaces", newWorkspaces!],
        ["tasks", newTasks!],
        ["selectedWorkspaceId", newSelectedId!],
      ]);
    } catch {
      setState(snapshot!);
      setPersistError("Failed to save workspace removal");
    }
  }, []);

  const reorderWorkspaces = useCallback((fromIndex: number, toIndex: number) => {
    let snapshot: AppState;
    let newWorkspaces: Workspace[];
    setState((prev) => {
      snapshot = prev;
      if (
        fromIndex === toIndex ||
        fromIndex < 0 ||
        toIndex < 0 ||
        fromIndex >= prev.workspaces.length ||
        toIndex >= prev.workspaces.length
      ) {
        newWorkspaces = prev.workspaces;
        return prev;
      }
      newWorkspaces = [...prev.workspaces];
      const [moved] = newWorkspaces.splice(fromIndex, 1);
      newWorkspaces.splice(toIndex, 0, moved);
      return { ...prev, workspaces: newWorkspaces };
    });
    if (newWorkspaces! === snapshot!.workspaces) return;
    void (async () => {
      try {
        await persist([["workspaces", newWorkspaces!]]);
      } catch {
        setState(snapshot!);
        setPersistError("Failed to save workspace order");
      }
    })();
  }, []);

  const selectWorkspace = useCallback((workspaceId: string) => {
    let snapshot: AppState;
    setState((prev) => {
      snapshot = prev;
      return { ...prev, selectedWorkspaceId: workspaceId };
    });
    if (!loadedWorkspaceIdsRef.current.has(workspaceId)) {
      setWorkspaceSwitching(true);
    }
    setTimeout(async () => {
      try {
        await persist([["selectedWorkspaceId", workspaceId]]);
      } catch {
        setState(snapshot!);
        setPersistError("Failed to save selection");
      }
    }, 0);
  }, []);

  const clearWorkspaceSwitching = useCallback((workspaceId?: string) => {
    if (workspaceId) loadedWorkspaceIdsRef.current.add(workspaceId);
    setWorkspaceSwitching(false);
  }, []);

  const addTask = useCallback(async (task: Task) => {
    let snapshot: AppState;
    let newTasks: Task[];
    setState((prev) => {
      snapshot = prev;
      newTasks = [...prev.tasks, task];
      return { ...prev, tasks: newTasks };
    });
    try {
      await persist([["tasks", newTasks!]]);
    } catch {
      setState(snapshot!);
      setPersistError("Failed to save task");
    }
  }, []);

  const removeTask = useCallback(async (taskId: string) => {
    let snapshot: AppState;
    let newTasks: Task[];
    setState((prev) => {
      snapshot = prev;
      newTasks = prev.tasks.filter((t) => t.id !== taskId);
      return { ...prev, tasks: newTasks };
    });
    try {
      await persist([["tasks", newTasks!]]);
    } catch {
      setState(snapshot!);
      setPersistError("Failed to save task removal");
    }
  }, []);

  const updateEditorApp = useCallback(async (editor: EditorApp) => {
    let prevEditor: EditorApp;
    setEditorAppState((current) => {
      prevEditor = current;
      return editor;
    });
    try {
      await persist([["editorApp", editor]]);
    } catch {
      setEditorAppState(prevEditor!);
      setPersistError("Failed to save editor preference");
    }
  }, []);

  const updateThemeId = useCallback(
    async (id: string) => {
      const prevTheme = themeId;
      const prevCustom = customColors;

      // First time Custom is selected (no saved colors yet): seed it from the
      // currently-active preset so the user starts from a familiar palette.
      let seeded = customColors;
      if (id === CUSTOM_THEME_ID && !seeded) {
        const source = themes.find((t) => t.id === prevTheme) ?? themes[0];
        seeded = { ...source.colors };
        setCustomColors(seeded);
      }

      setThemeIdState(id);
      applyTheme(id, id === CUSTOM_THEME_ID ? (seeded ?? undefined) : undefined);

      try {
        const entries: [string, unknown][] = [["themeId", id]];
        if (id === CUSTOM_THEME_ID && !prevCustom && seeded) {
          entries.push(["customTheme", seeded]);
        }
        await persist(entries);
      } catch {
        setThemeIdState(prevTheme);
        setCustomColors(prevCustom);
        applyTheme(prevTheme, prevTheme === CUSTOM_THEME_ID ? (prevCustom ?? undefined) : undefined);
        setPersistError("Failed to save theme preference");
      }
    },
    [themeId, customColors],
  );

  const updateCustomColors = useCallback(
    async (colors: Record<string, string>) => {
      const prevCustom = customColors;
      setCustomColors(colors);
      applyTheme(CUSTOM_THEME_ID, colors);
      try {
        await persist([["customTheme", colors]]);
      } catch {
        setCustomColors(prevCustom);
        applyTheme(CUSTOM_THEME_ID, prevCustom ?? undefined);
        setPersistError("Failed to save theme preference");
      }
    },
    [customColors],
  );

  const selectedWorkspace = useMemo(
    () => state.workspaces.find((w) => w.id === state.selectedWorkspaceId),
    [state.workspaces, state.selectedWorkspaceId],
  );
  const selectedTasks = useMemo(
    () => state.tasks.filter((t) => t.workspaceId === state.selectedWorkspaceId),
    [state.tasks, state.selectedWorkspaceId],
  );

  return {
    state,
    loading,
    editorApp,
    themeId,
    customColors,
    selectedWorkspace,
    selectedTasks,
    workspaceSwitching,
    persistError,
    dismissPersistError,
    updateSetup,
    addWorkspace,
    updateWorkspace,
    removeWorkspace,
    reorderWorkspaces,
    selectWorkspace,
    clearWorkspaceSwitching,
    addTask,
    removeTask,
    updateEditorApp,
    updateThemeId,
    updateCustomColors,
  };
}
