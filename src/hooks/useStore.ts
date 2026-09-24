import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  loadState,
  restoreKeychainSecrets,
  loadEditorApp,
  loadAgentViews,
  loadThemeId,
  loadCustomColors,
  loadSidebarCollapsed,
  persist,
} from "../services/store";
import {
  AgentId,
  AgentView,
  AgentViews,
  AppState,
  DEFAULT_AGENT_VIEWS,
  DEFAULT_STATE,
  EditorApp,
  VaultConfig,
  Workspace,
} from "../types";
import { Operations, CreateTaskInput, DeleteOptions, TaskReady } from "../services/operations";
import { applyTheme, themes, CUSTOM_THEME_ID } from "../themes";

export function useStore() {
  const [state, setState] = useState<AppState>(DEFAULT_STATE);
  const [loading, setLoading] = useState(true);
  const [editorApp, setEditorAppState] = useState<EditorApp>("cursor");
  const [agentViews, setAgentViews] = useState<AgentViews>(DEFAULT_AGENT_VIEWS);
  const agentViewsRef = useRef(agentViews);
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

  const [operations] = useState(
    () =>
      new Operations(
        () => stateRef.current,
        commit,
        () => editorAppRef.current
      )
  );

  const report = useCallback(async <T>(action: Promise<T>): Promise<T> => {
    try {
      return await action;
    } catch (error) {
      setPersistError(error instanceof Error ? error.message : "Operation failed");
      throw error;
    }
  }, []);

  useEffect(() => {
    Promise.all([
      loadState(setKeychainError),
      loadEditorApp(),
      loadThemeId(),
      loadCustomColors(),
      loadSidebarCollapsed(),
      loadAgentViews(),
    ])
      .then(([s, editor, theme, custom, collapsed, views]) => {
        agentViewsRef.current = views;
        setAgentViews(views);
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
      const restored = await operations.refresh(restoreKeychainSecrets);
      setKeychainError(null);
      return restored;
    } catch (error) {
      setKeychainError(String(error));
      throw error;
    } finally {
      setKeychainRetrying(false);
    }
  }, [operations]);

  const dismissPersistError = useCallback(() => setPersistError(null), []);

  const updateSetup = useCallback(
    (setup: AppState["setup"]) => report(operations.change(() => ({ setup }))),
    [operations, report]
  );

  const updateVault = useCallback(
    (vault: VaultConfig) => report(operations.change(() => ({ vault }))),
    [operations, report]
  );

  const addWorkspace = useCallback(
    (workspace: Workspace) => report(operations.addWorkspace(workspace)),
    [operations, report]
  );

  const updateWorkspace = useCallback(
    (
      id: string,
      updates: Partial<Pick<Workspace, "name" | "linearApiKey" | "linearOrgUrlKey" | "repos">>
    ) => report(operations.updateWorkspace(id, updates)),
    [operations, report]
  );

  const removeWorkspace = useCallback(
    (id: string, options?: DeleteOptions) =>
      report(operations.deleteWorkspace(id, options)).then((result) => {
        loadedWorkspaceIdsRef.current.delete(id);
        return result;
      }),
    [operations, report]
  );

  const reorderWorkspaces = useCallback(
    (fromIndex: number, toIndex: number) => {
      void report(
        operations.change((snapshot) => {
          const workspaces = [...snapshot.workspaces];
          if (
            fromIndex < 0 ||
            toIndex < 0 ||
            fromIndex >= workspaces.length ||
            toIndex >= workspaces.length
          )
            return {};
          const [moved] = workspaces.splice(fromIndex, 1);
          workspaces.splice(toIndex, 0, moved);
          return { workspaces };
        })
      ).catch(() => {});
    },
    [operations, report]
  );

  const selectWorkspace = useCallback(
    (workspaceId: string) => {
      setWorkspaceSwitching(!loadedWorkspaceIdsRef.current.has(workspaceId));
      void report(
        operations.change((snapshot) => ({
          selectedWorkspaceId: snapshot.workspaces.some((workspace) => workspace.id === workspaceId)
            ? workspaceId
            : snapshot.selectedWorkspaceId,
        }))
      ).catch(() => {});
    },
    [operations, report]
  );

  const clearWorkspaceSwitching = useCallback((workspaceId?: string) => {
    if (workspaceId) loadedWorkspaceIdsRef.current.add(workspaceId);
    setWorkspaceSwitching(false);
  }, []);

  const createTask = useCallback(
    (input: CreateTaskInput, progress?: (message: string) => void, onReady?: TaskReady) =>
      report(operations.createTask(input, progress, onReady)),
    [operations, report]
  );

  const removeTask = useCallback(
    (id: string, options: DeleteOptions) => report(operations.deleteTask(id, options)),
    [operations, report]
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

  const updateAgentView = useCallback(async (agent: AgentId, view: AgentView) => {
    const previous = agentViewsRef.current;
    const next = { ...previous, [agent]: view };
    agentViewsRef.current = next;
    setAgentViews(next);
    try {
      await persist([["agentViews", next]]);
    } catch {
      agentViewsRef.current = previous;
      setAgentViews(previous);
      setPersistError("Failed to save the agent view preference");
    }
  }, []);

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
    agentViews,
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
    createTask,
    operations,
    removeTask,
    updateEditorApp,
    updateAgentView,
    sidebarCollapsed,
    toggleSidebarCollapsed,
    updateThemeId,
    updateCustomColors,
  };
}
