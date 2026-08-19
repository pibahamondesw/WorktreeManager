import { useState, useMemo, useCallback } from "react";
import { SetupWizard } from "./components/setup/SetupWizard";
import { WorkspaceList } from "./components/sidebar/WorkspaceList";
import { WorktreeList } from "./components/worktree/WorktreeList";
import { QuickSearchModal } from "./components/search/QuickSearchModal";
import { SpinnerIcon } from "./components/ui/Icons";
import { ErrorBoundary } from "./components/ui/ErrorBoundary";
import { useStore } from "./hooks/useStore";
import { enableVault } from "./services/vault";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { useUpdater } from "./hooks/useUpdater";
import { useWindowDrag } from "./hooks/useWindowDrag";
import { withScope } from "./search/query";
import { Task } from "./types";

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
    updateThemeId,
    updateCustomColors,
  } = useStore();

  const [showAddWorkspace, setShowAddWorkspace] = useState(false);
  const [search, setSearch] = useState<{ open: boolean; query: string }>({
    open: false,
    query: "",
  });
  const [revealTaskId, setRevealTaskId] = useState<string | null>(null);

  const openSearch = useCallback(
    (scoped: boolean) => {
      const name = selectedWorkspace?.name;
      setSearch({ open: true, query: scoped && name ? withScope("", name) : "" });
    },
    [selectedWorkspace?.name]
  );

  const handleReveal = useCallback(
    (task: Task) => {
      if (task.workspaceId !== state.selectedWorkspaceId) selectWorkspace(task.workspaceId);
      setRevealTaskId(task.id);
    },
    [state.selectedWorkspaceId, selectWorkspace]
  );

  const handleSelectWorkspace = useCallback(
    (workspaceId: string) => {
      if (workspaceId === state.selectedWorkspaceId) return;
      selectWorkspace(workspaceId);
    },
    [state.selectedWorkspaceId, selectWorkspace]
  );

  const defaultLinearApiKey = useMemo(() => {
    const lastWithKey = [...state.workspaces].reverse().find((w) => w.linearApiKey);
    return lastWithKey?.linearApiKey ?? state.setup.linearApiKey ?? null;
  }, [state.workspaces, state.setup.linearApiKey]);

  useKeyboardShortcuts({
    p: {
      handler: () => setShowAddWorkspace(true),
      enabled: state.setup.isComplete && !search.open,
    },
    "meta+shift+r": { handler: () => window.location.reload() },
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
    <div className="flex h-full relative">
      {/* Full-width drag region at the very top for window dragging */}
      <div className="absolute top-0 left-0 right-0 h-[32px] z-[5]" data-drag-region />
      {/* Titlebar divider — spans full width at bottom of macOS traffic lights area */}
      <div className="absolute top-[32px] left-0 right-0 h-px bg-border z-10 pointer-events-none" />
      {persistError && (
        <div className="absolute top-[39px] left-0 right-0 z-20 px-4 py-2 bg-danger/10 border-b border-danger/20 flex items-center justify-between">
          <span className="text-xs text-danger">{persistError}</span>
          <button
            onClick={dismissPersistError}
            className="text-xs text-danger/70 hover:text-danger transition-colors cursor-pointer ml-4 flex-shrink-0"
          >
            Dismiss
          </button>
        </div>
      )}
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
        />
      </ErrorBoundary>
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
        />
      </ErrorBoundary>
      <QuickSearchModal
        open={search.open}
        onClose={() => setSearch((s) => ({ ...s, open: false }))}
        initialQuery={search.query}
        tasks={state.tasks}
        workspaces={state.workspaces}
        selectedWorkspaceId={state.selectedWorkspaceId}
        editorApp={editorApp}
        onReveal={handleReveal}
      />
    </div>
  );
}

export default App;
