import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import { SetupWizard } from "./components/setup/SetupWizard";
import { WorkspaceList } from "./components/sidebar/WorkspaceList";
import { WorktreeList } from "./components/worktree/WorktreeList";
import { QuickSearchModal } from "./components/search/QuickSearchModal";
import { DoctorModal } from "./components/doctor/DoctorModal";
import { SpinnerIcon } from "./components/ui/Icons";
import { ErrorBoundary } from "./components/ui/ErrorBoundary";
import { useStore } from "./hooks/useStore";
import { enableVault } from "./services/vault";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { useUpdater } from "./hooks/useUpdater";
import { useLinearOrgKeyBackfill } from "./hooks/useLinearOrgKeyBackfill";
import { useDoctor } from "./hooks/useDoctor";
import { useWindowDrag } from "./hooks/useWindowDrag";
import { useNavigationHistory } from "./hooks/useNavigationHistory";
import { useOpenTask } from "./hooks/useOpenTask";
import { NavigationEntry } from "./navigation/history";
import { withScope } from "./search/query";
import { CheckSeverity, DoctorConfig } from "./services/doctor";
import { Task } from "./types";
import { listen } from "@tauri-apps/api/event";
import { editorPresentation } from "./services/codeEditor";

function App() {
  const {
    state,
    loading,
    editorApp,
    themeId,
    customColors,
    selectedWorkspace,
    selectedTasks,
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
    workspaceSwitching,
    addTask,
    removeTask,
    updateEditorApp,
    sidebarCollapsed,
    toggleSidebarCollapsed,
    updateThemeId,
    updateCustomColors,
  } = useStore();

  const [showAddWorkspace, setShowAddWorkspace] = useState(false);
  const [search, setSearch] = useState<{ open: boolean; query: string }>({
    open: false,
    query: "",
  });
  const [revealTaskId, setRevealTaskId] = useState<string | null>(null);
  const [showDoctor, setShowDoctor] = useState(false);
  const [doctorAlertDismissed, setDoctorAlertDismissed] = useState(false);

  useEffect(() => {
    editorPresentation.reset();
  }, []);

  const openSearch = useCallback(
    (scoped: boolean) => {
      const name = selectedWorkspace?.name;
      setSearch({ open: true, query: scoped && name ? withScope("", name) : "" });
    },
    [selectedWorkspace?.name]
  );

  const showTask = useCallback(
    (task: Task) => {
      if (task.workspaceId !== state.selectedWorkspaceId) selectWorkspace(task.workspaceId);
      setRevealTaskId(task.id);
    },
    [state.selectedWorkspaceId, selectWorkspace]
  );

  const isNavigable = useCallback(
    (entry: NavigationEntry) =>
      entry.kind === "task"
        ? state.tasks.some((t) => t.id === entry.taskId)
        : state.workspaces.some((w) => w.id === entry.workspaceId),
    [state.tasks, state.workspaces]
  );

  const { recordTaskVisit, recordWorkspaceVisit, ...history } = useNavigationHistory({
    isNavigable,
    onNavigate: (entry) => {
      if (entry.kind === "task") {
        const task = state.tasks.find((t) => t.id === entry.taskId);
        if (task) restoreTask(task);
      } else {
        closeTask();
        if (entry.workspaceId !== state.selectedWorkspaceId) selectWorkspace(entry.workspaceId);
      }
    },
  });

  const initialVisitRecorded = useRef(false);
  useEffect(() => {
    if (loading || initialVisitRecorded.current || !state.selectedWorkspaceId) return;
    initialVisitRecorded.current = true;
    recordWorkspaceVisit(state.selectedWorkspaceId);
  }, [loading, state.selectedWorkspaceId, recordWorkspaceVisit]);

  const handleReveal = useCallback(
    (task: Task) => {
      showTask(task);
      recordTaskVisit(task);
    },
    [showTask, recordTaskVisit]
  );

  const { openedTask, openTask, closeTask, restoreTask } = useOpenTask({
    editorApp,
    workspaces: state.workspaces,
    tasks: state.tasks,
    recordTaskVisit,
    showTask,
  });

  useEffect(() => {
    const unlisten = listen<string>("editor-navigate", ({ payload }) => {
      if (payload === "back") closeTask();
      if (payload === "search") openSearch(false);
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [closeTask, openSearch]);

  const handleSelectWorkspace = useCallback(
    (workspaceId: string) => {
      if (workspaceId === state.selectedWorkspaceId) return;
      closeTask();
      selectWorkspace(workspaceId);
      recordWorkspaceVisit(workspaceId);
    },
    [state.selectedWorkspaceId, selectWorkspace, recordWorkspaceVisit, closeTask]
  );

  const defaultLinearApiKey = useMemo(() => {
    const lastWithKey = [...state.workspaces].reverse().find((w) => w.linearApiKey);
    return lastWithKey?.linearApiKey ?? state.setup.linearApiKey ?? null;
  }, [state.workspaces, state.setup.linearApiKey]);

  // Nothing is checked before setup completes: there are no repos or workspace keys to check yet,
  // and the wizard validates the Linear key it collects inline.
  const doctorConfig = useMemo<DoctorConfig | null>(() => {
    if (loading || !state.setup.isComplete) return null;
    return {
      editor: editorApp,
      keychainError,
      vaultEnabled: state.vault.enabled,
      repoPaths: [...new Set(state.workspaces.flatMap((w) => w.repos.map((r) => r.localPath)))],
      linearKeys: state.workspaces.map((w) => ({
        label: w.name,
        key: w.linearApiKey ?? null,
      })),
    };
  }, [
    loading,
    state.setup.isComplete,
    state.vault.enabled,
    state.workspaces,
    editorApp,
    keychainError,
  ]);

  const {
    report: doctorReport,
    running: doctorRunning,
    recheck: recheckDoctor,
  } = useDoctor(doctorConfig);

  const doctorSeverity: CheckSeverity | null = doctorReport
    ? doctorReport.errors > 0
      ? "error"
      : doctorReport.warnings > 0
        ? "warning"
        : "ok"
    : null;

  const missingDependencies = (doctorReport?.checks ?? [])
    .filter((c) => c.scope === "app" && c.severity === "error")
    .map((c) => c.label);
  const showDoctorAlert = missingDependencies.length > 0 && !doctorAlertDismissed;

  useKeyboardShortcuts({
    p: {
      handler: () => setShowAddWorkspace(true),
      enabled: state.setup.isComplete && !search.open,
    },
    "meta+shift+r": { handler: () => window.location.reload() },
    "[": {
      handler: toggleSidebarCollapsed,
      enabled: state.setup.isComplete && !search.open,
    },
    "meta+k": {
      handler: () => openSearch(false),
      enabled: state.setup.isComplete,
      inTextFields: true,
    },
    "meta+f": {
      handler: () => openSearch(true),
      enabled: state.setup.isComplete,
      inTextFields: true,
    },
    "meta+ArrowLeft": {
      handler: history.back,
      enabled: state.setup.isComplete && !search.open && history.canGoBack,
    },
    "meta+ArrowRight": {
      handler: history.forward,
      enabled: state.setup.isComplete && !search.open && history.canGoForward,
    },
    ...Object.fromEntries(
      Array.from({ length: 10 }, (_, i) => [
        `meta+${i}`,
        {
          handler: () => handleSelectWorkspace(state.workspaces[i].id),
          enabled: state.setup.isComplete && !search.open && i < state.workspaces.length,
        },
      ])
    ),
  });

  useUpdater();
  useWindowDrag();
  useLinearOrgKeyBackfill(state.workspaces, updateWorkspace);

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full" data-drag-region>
        <SpinnerIcon size={24} className="text-text-muted" />
      </div>
    );
  }

  if (!state.setup.isComplete) {
    return (
      <SetupWizard
        initialSetup={state.setup}
        onComplete={(setup, { enableVault: wantsVault }) => {
          void updateSetup(setup);
          // Best-effort here — the sidebar's vault settings are the recovery path.
          if (wantsVault) {
            enableVault()
              .then(updateVault)
              .catch(() => undefined);
          }
        }}
      />
    );
  }

  return (
    <div className="flex flex-col h-full relative">
      <div className="h-[33px] shrink-0 bg-bg-secondary border-b border-border" data-drag-region />
      {(persistError || showDoctorAlert) && (
        <div className="shrink-0 bg-bg-secondary">
          {persistError && (
            <div className="px-4 py-2 bg-danger/10 border-b border-danger/20 flex items-center justify-between">
              <span className="text-xs text-danger">{persistError}</span>
              <button
                onClick={dismissPersistError}
                className="text-xs text-danger/70 hover:text-danger transition-colors cursor-pointer ml-4 flex-shrink-0"
              >
                Dismiss
              </button>
            </div>
          )}
          {showDoctorAlert && (
            <div
              role="status"
              className="px-4 py-2 bg-warning/10 border-b border-warning/20 flex items-center justify-between gap-3"
            >
              <span className="min-w-0 text-xs text-warning break-words">
                WorktreeManager needs attention: {missingDependencies.join(", ")}
              </span>
              <span className="flex items-center gap-3 ml-4 flex-shrink-0">
                <button
                  onClick={() => setShowDoctor(true)}
                  className="text-xs text-warning underline hover:no-underline transition-all cursor-pointer"
                >
                  Review
                </button>
                <button
                  onClick={() => setDoctorAlertDismissed(true)}
                  className="text-xs text-warning/70 hover:text-warning transition-colors cursor-pointer"
                >
                  Dismiss
                </button>
              </span>
            </div>
          )}
        </div>
      )}
      <div className="flex flex-1 min-h-0">
        {!sidebarCollapsed && (
          <ErrorBoundary fallbackClassName="w-60 h-full bg-bg-secondary border-r border-border">
            <WorkspaceList
              workspaces={state.workspaces}
              tasks={state.tasks}
              selectedWorkspaceId={state.selectedWorkspaceId}
              onSelect={handleSelectWorkspace}
              onAdd={addWorkspace}
              onUpdate={updateWorkspace}
              onRemove={removeWorkspace}
              onReorder={reorderWorkspaces}
              showAddExternal={showAddWorkspace}
              onCloseAddExternal={() => setShowAddWorkspace(false)}
              themeId={themeId}
              onThemeChange={updateThemeId}
              customColors={customColors}
              onCustomColorsChange={updateCustomColors}
              defaultLinearApiKey={defaultLinearApiKey}
              vault={state.vault}
              onVaultChange={updateVault}
              onCollapse={toggleSidebarCollapsed}
              doctorSeverity={doctorSeverity}
              onOpenDoctor={() => setShowDoctor(true)}
            />
          </ErrorBoundary>
        )}
        <ErrorBoundary fallbackClassName="flex-1">
          <WorktreeList
            tasks={selectedTasks}
            workspace={selectedWorkspace}
            vault={state.vault}
            onTaskCreated={addTask}
            onTaskDeleted={removeTask}
            editorApp={editorApp}
            onEditorChange={updateEditorApp}
            workspaceSwitching={workspaceSwitching}
            onWorkspaceReady={clearWorkspaceSwitching}
            onOpenSearch={() => openSearch(false)}
            searchOpen={search.open}
            revealTaskId={revealTaskId}
            onRevealHandled={() => setRevealTaskId(null)}
            openedTask={openedTask}
            onOpenTask={openTask}
            onCloseTask={closeTask}
            canGoBack={history.canGoBack}
            canGoForward={history.canGoForward}
            onGoBack={history.back}
            onGoForward={history.forward}
            sidebarCollapsed={sidebarCollapsed}
            onExpandSidebar={toggleSidebarCollapsed}
          />
        </ErrorBoundary>
      </div>
      <QuickSearchModal
        open={search.open}
        onClose={() => setSearch((s) => ({ ...s, open: false }))}
        initialQuery={search.query}
        tasks={state.tasks}
        workspaces={state.workspaces}
        selectedWorkspaceId={state.selectedWorkspaceId}
        historyEntries={history.entries}
        onReveal={handleReveal}
        onOpenTask={openTask}
      />
      <DoctorModal
        open={showDoctor}
        onClose={() => setShowDoctor(false)}
        report={doctorReport}
        running={doctorRunning || keychainRetrying}
        onRecheck={async () => {
          if (!keychainError || !doctorConfig) {
            await recheckDoctor();
            return;
          }
          try {
            const restored = await retryKeychain();
            await recheckDoctor({
              ...doctorConfig,
              keychainError: null,
              linearKeys: restored.workspaces.map((workspace) => ({
                label: workspace.name,
                key: workspace.linearApiKey ?? null,
              })),
            });
          } catch (error) {
            await recheckDoctor({ ...doctorConfig, keychainError: String(error) });
          }
        }}
      />
    </div>
  );
}

export default App;
