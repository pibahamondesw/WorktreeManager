import { setShortcutOverrides, syncNativeShortcuts } from "../shortcuts/runtime";
import { VaultAgent } from "../services/vault";
import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { compareTaskPins } from "../utils";
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
  AppState,
  DEFAULT_STATE,
  EditorApp,
  VaultConfig,
  Workspace,
} from "../types";
import { Operations, CreateTaskInput, DeleteOptions, TaskReady } from "../services/operations";
import { applyTheme, presetFor, CUSTOM_THEME_ID, SYSTEM_THEME_ID } from "../themes";

import { defaultInterfacePreferences, PreferenceSession } from "../services/preferences";

export function useStore() {
  const [state, setState] = useState<AppState>(DEFAULT_STATE);
  const [loading, setLoading] = useState(true);
  const [preferences, setPreferences] = useState(defaultInterfacePreferences);
  const {
    editorApp,
    agentViews,
    themeId,
    customTheme: customColors,
    sidebarCollapsed,
  } = preferences;
  const [workspaceSwitching, setWorkspaceSwitching] = useState(true);
  const [persistError, setPersistError] = useState<string | null>(null);
  const [keychainRetrying, setKeychainRetrying] = useState(false);
  const [keychainError, setKeychainError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<Error | null>(null);
  // Workspaces whose data finished loading at least once this session: switching
  // back to them shows the in-memory data immediately instead of the skeleton.
  const loadedWorkspaceIdsRef = useRef(new Set<string>());

  const stateRef = useRef(state);
  const [preferenceSession] = useState(
    () =>
      new PreferenceSession(
        (next) => {
          setPreferences(next);
          applyTheme(
            next.themeId,
            next.themeId === CUSTOM_THEME_ID ? (next.customTheme ?? undefined) : undefined
          );
        },
        setPersistError,
        persist
      )
  );

  const commit = useCallback((next: AppState) => {
    stateRef.current = next;
    setState(next);
  }, []);

  const [operations] = useState(
    () =>
      new Operations(
        () => stateRef.current,
        commit,
        () => preferenceSession.value.editorApp
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
      .then(async ([s, editor, theme, custom, collapsed, views]) => {
        await syncNativeShortcuts(s.shortcutOverrides ?? {});
        setShortcutOverrides(s.shortcutOverrides ?? {});
        commit(s);
        preferenceSession.replace({
          editorApp: editor,
          agentViews: views,
          themeId: theme,
          customTheme: custom,
          sidebarCollapsed: collapsed,
        });
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
  }, [commit, preferenceSession]);

  useEffect(() => {
    if (themeId !== SYSTEM_THEME_ID) return;
    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    if (!media) return;
    const followSystem = () => applyTheme(SYSTEM_THEME_ID);
    media.addEventListener("change", followSystem);
    return () => media.removeEventListener("change", followSystem);
  }, [themeId]);

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
    (vault: VaultConfig) => report(operations.updateVault(vault)),
    [operations, report]
  );

  const repairVaultAgents = useCallback(
    (agent?: VaultAgent) => report(operations.repairVaultAgents(agent)),
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
      operations.deleteWorkspace(id, options).then((result) => {
        loadedWorkspaceIdsRef.current.delete(id);
        return result;
      }),
    [operations]
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
    (id: string, options: DeleteOptions) => operations.deleteTask(id, options),
    [operations]
  );

  const updateEditorApp = useCallback(
    (editor: EditorApp) =>
      preferenceSession.update({ editorApp: editor }, "Failed to save editor preference"),
    [preferenceSession]
  );

  const updateAgentView = useCallback(
    (agent: AgentId, view: AgentView) =>
      preferenceSession.update(
        (current) => ({ agentViews: { ...current.agentViews, [agent]: view } }),
        "Failed to save the agent view preference"
      ),
    [preferenceSession]
  );

  const toggleSidebarCollapsed = useCallback(() => {
    void preferenceSession.update(
      (current) => ({ sidebarCollapsed: !current.sidebarCollapsed }),
      "Failed to save sidebar preference"
    );
  }, [preferenceSession]);

  const updateThemeId = useCallback(
    (id: string) =>
      preferenceSession.update(
        (current) => ({
          themeId: id,
          ...(id === CUSTOM_THEME_ID && !current.customTheme
            ? { customTheme: { ...presetFor(current.themeId).colors } }
            : {}),
        }),
        "Failed to save theme preference"
      ),
    [preferenceSession]
  );

  const updateCustomColors = useCallback(
    (colors: Record<string, string>) =>
      preferenceSession.update({ customTheme: colors }, "Failed to save theme preference"),
    [preferenceSession]
  );

  const selectedWorkspace = useMemo(
    () => state.workspaces.find((w) => w.id === state.selectedWorkspaceId),
    [state.workspaces, state.selectedWorkspaceId]
  );
  const selectedTasks = useMemo(
    () =>
      state.tasks.filter((t) => t.workspaceId === state.selectedWorkspaceId).sort(compareTaskPins),
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
    repairVaultAgents,
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
