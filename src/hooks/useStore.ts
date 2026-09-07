import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  loadState,
  restoreKeychainSecrets,
  loadEditorApp,
  loadThemeId,
  loadCustomColors,
  loadSidebarCollapsed,
  persist,
} from "../services/store";
import { AppState, DEFAULT_STATE, EditorApp, Task, VaultConfig, Workspace } from "../types";
import { applyTheme, themes, CUSTOM_THEME_ID } from "../themes";

export function useStore() {
  const [state, setState] = useState<AppState>(DEFAULT_STATE);
  const [loading, setLoading] = useState(true);
  const [editorApp, setEditorAppState] = useState<EditorApp>("cursor");
  const [themeId, setThemeIdState] = useState("default");
  const [customColors, setCustomColors] = useState<Record<string, string> | null>(null);
  const [sidebarCollapsed, setSidebarCollapsedState] = useState(false);
  const [workspaceSwitching, setWorkspaceSwitching] = useState(true);
  const [persistError, setPersistError] = useState<string | null>(null);
  const [keychainRetrying, setKeychainRetrying] = useState(false);
  const [keychainError, setKeychainError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<Error | null>(null);
  // Workspaces whose data finished loading at least once this session: switching
  // back to them shows the in-memory data immediately instead of the skeleton.
  const loadedWorkspaceIdsRef = useRef(new Set<string>());

  const stateRef = useRef(state);
  const editorAppRef = useRef(editorApp);

  const commit = useCallback((next: AppState) => {
    stateRef.current = next;
    setState(next);
  }, []);

  const commitEditorApp = useCallback((next: EditorApp) => {
    editorAppRef.current = next;
    setEditorAppState(next);
  }, []);

  useEffect(() => {
    Promise.all([
      loadState(setKeychainError),
      loadEditorApp(),
      loadThemeId(),
      loadCustomColors(),
      loadSidebarCollapsed(),
    ])
      .then(([s, editor, theme, custom, collapsed]) => {
        commit(s);
        commitEditorApp(editor);
        setThemeIdState(theme);
        setCustomColors(custom);
        setSidebarCollapsedState(collapsed);
        applyTheme(theme, theme === CUSTOM_THEME_ID ? (custom ?? undefined) : undefined);
        setLoading(false);

        const basePaths = [
          ...new Set(s.workspaces.flatMap((w) => w.repos.map((r) => r.worktreeBasePath))),
        ];
        void invoke("cleanup_claude_json_stale", { basePaths }).catch(() => {});

        // Self-heal an enabled vault: recreate it if the folder went missing and
        // keep it registered in Obsidian. Best-effort, never blocks startup.
        if (s.vault.enabled && s.vault.path) {
          void invoke("ensure_vault", { vaultPath: s.vault.path }).catch(() => {});
        }
      })
      .catch((e: unknown) => {
        setLoadError(e instanceof Error ? e : new Error(String(e)));
      });
  }, [commit, commitEditorApp]);

  const retryKeychain = useCallback(async () => {
    setKeychainRetrying(true);
    try {
      const restored = await restoreKeychainSecrets(() => stateRef.current);
      commit(restored);
      setKeychainError(null);
      return restored;
    } catch (error) {
      setKeychainError(String(error));
      throw error;
    } finally {
      setKeychainRetrying(false);
    }
  }, [commit]);

  const dismissPersistError = useCallback(() => setPersistError(null), []);

  const updateSetup = useCallback(
    async (setup: AppState["setup"]) => {
      const snapshot = stateRef.current;
      commit({ ...snapshot, setup });
      try {
        await persist([["setup", setup]]);
      } catch {
        commit(snapshot);
        setPersistError("Failed to save setup changes");
      }
    },
    [commit]
  );

  const updateVault = useCallback(
    async (vault: VaultConfig) => {
      const snapshot = stateRef.current;
      commit({ ...snapshot, vault });
      try {
        await persist([["vault", vault]]);
      } catch {
        commit(snapshot);
        setPersistError("Failed to save vault settings");
      }
    },
    [commit]
  );

  const addWorkspace = useCallback(
    async (workspace: Workspace) => {
      const snapshot = stateRef.current;
      const newWorkspaces = [...snapshot.workspaces, workspace];
      commit({ ...snapshot, workspaces: newWorkspaces, selectedWorkspaceId: workspace.id });
      try {
        await persist([
          ["workspaces", newWorkspaces],
          ["selectedWorkspaceId", workspace.id],
        ]);
      } catch {
        commit(snapshot);
        setPersistError("Failed to save workspace");
      }
    },
    [commit]
  );

  const updateWorkspace = useCallback(
    async (
      workspaceId: string,
      updates: Partial<Pick<Workspace, "name" | "linearApiKey" | "linearOrgUrlKey" | "repos">>
    ) => {
      const snapshot = stateRef.current;
      const newWorkspaces = snapshot.workspaces.map((w) =>
        w.id === workspaceId ? { ...w, ...updates } : w
      );
      commit({ ...snapshot, workspaces: newWorkspaces });
      try {
        await persist([["workspaces", newWorkspaces]]);
      } catch {
        commit(snapshot);
        setPersistError("Failed to save workspace changes");
      }
    },
    [commit]
  );

  const removeWorkspace = useCallback(
    async (workspaceId: string) => {
      loadedWorkspaceIdsRef.current.delete(workspaceId);
      const snapshot = stateRef.current;
      const newWorkspaces = snapshot.workspaces.filter((w) => w.id !== workspaceId);
      const newTasks = snapshot.tasks.filter((t) => t.workspaceId !== workspaceId);
      const newSelectedId =
        snapshot.selectedWorkspaceId === workspaceId
          ? (newWorkspaces[0]?.id ?? null)
          : snapshot.selectedWorkspaceId;
      commit({
        ...snapshot,
        workspaces: newWorkspaces,
        tasks: newTasks,
        selectedWorkspaceId: newSelectedId,
      });
      try {
        await persist([
          ["workspaces", newWorkspaces],
          ["tasks", newTasks],
          ["selectedWorkspaceId", newSelectedId],
        ]);
      } catch {
        commit(snapshot);
        setPersistError("Failed to save workspace removal");
      }
    },
    [commit]
  );

  const reorderWorkspaces = useCallback(
    (fromIndex: number, toIndex: number) => {
      const snapshot = stateRef.current;
      if (
        fromIndex === toIndex ||
        fromIndex < 0 ||
        toIndex < 0 ||
        fromIndex >= snapshot.workspaces.length ||
        toIndex >= snapshot.workspaces.length
      ) {
        return;
      }
      const newWorkspaces = [...snapshot.workspaces];
      const [moved] = newWorkspaces.splice(fromIndex, 1);
      newWorkspaces.splice(toIndex, 0, moved);
      commit({ ...snapshot, workspaces: newWorkspaces });
      void (async () => {
        try {
          await persist([["workspaces", newWorkspaces]]);
        } catch {
          commit(snapshot);
          setPersistError("Failed to save workspace order");
        }
      })();
    },
    [commit]
  );

  const selectWorkspace = useCallback(
    (workspaceId: string) => {
      const snapshot = stateRef.current;
      commit({ ...snapshot, selectedWorkspaceId: workspaceId });
      setWorkspaceSwitching(!loadedWorkspaceIdsRef.current.has(workspaceId));
      setTimeout(async () => {
        try {
          await persist([["selectedWorkspaceId", workspaceId]]);
        } catch {
          commit(snapshot);
          setPersistError("Failed to save selection");
        }
      }, 0);
    },
    [commit]
  );

  const clearWorkspaceSwitching = useCallback((workspaceId?: string) => {
    if (workspaceId) loadedWorkspaceIdsRef.current.add(workspaceId);
    setWorkspaceSwitching(false);
  }, []);

  const addTask = useCallback(
    async (task: Task) => {
      const snapshot = stateRef.current;
      const newTasks = [...snapshot.tasks, task];
      commit({ ...snapshot, tasks: newTasks });
      try {
        await persist([["tasks", newTasks]]);
      } catch {
        commit(snapshot);
        setPersistError("Failed to save task");
      }
    },
    [commit]
  );

  const removeTask = useCallback(
    async (taskId: string) => {
      const snapshot = stateRef.current;
      const newTasks = snapshot.tasks.filter((t) => t.id !== taskId);
      commit({ ...snapshot, tasks: newTasks });
      try {
        await persist([["tasks", newTasks]]);
      } catch {
        commit(snapshot);
        setPersistError("Failed to save task removal");
      }
    },
    [commit]
  );

  const updateEditorApp = useCallback(
    async (editor: EditorApp) => {
      const prevEditor = editorAppRef.current;
      commitEditorApp(editor);
      try {
        await persist([["editorApp", editor]]);
      } catch {
        commitEditorApp(prevEditor);
        setPersistError("Failed to save editor preference");
      }
    },
    [commitEditorApp]
  );

  const toggleSidebarCollapsed = useCallback(() => {
    setSidebarCollapsedState((prev) => {
      const next = !prev;
      void persist([["sidebarCollapsed", next]]).catch(() => {});
      return next;
    });
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
        applyTheme(
          prevTheme,
          prevTheme === CUSTOM_THEME_ID ? (prevCustom ?? undefined) : undefined
        );
        setPersistError("Failed to save theme preference");
      }
    },
    [themeId, customColors]
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
    [customColors]
  );

  const selectedWorkspace = useMemo(
    () => state.workspaces.find((w) => w.id === state.selectedWorkspaceId),
    [state.workspaces, state.selectedWorkspaceId]
  );
  const selectedTasks = useMemo(
    () => state.tasks.filter((t) => t.workspaceId === state.selectedWorkspaceId),
    [state.tasks, state.selectedWorkspaceId]
  );

  if (loadError) throw loadError;

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
    keychainError,
    keychainRetrying,
    retryKeychain,
    dismissPersistError,
    updateSetup,
    updateVault,
    addWorkspace,
    updateWorkspace,
    removeWorkspace,
    reorderWorkspaces,
    selectWorkspace,
    clearWorkspaceSwitching,
    addTask,
    removeTask,
    updateEditorApp,
    sidebarCollapsed,
    toggleSidebarCollapsed,
    updateThemeId,
    updateCustomColors,
  };
}
