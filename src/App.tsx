import { useState, useMemo, useCallback } from "react";
import { SetupWizard } from "./components/setup/SetupWizard";
import { WorkspaceList } from "./components/sidebar/WorkspaceList";
import { WorktreeList } from "./components/worktree/WorktreeList";
import { SpinnerIcon } from "./components/ui/Icons";
import { ErrorBoundary } from "./components/ui/ErrorBoundary";
import { useStore } from "./hooks/useStore";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";

function App() {
  const {
    state,
    loading,
    editorApp,
    themeId,
    selectedWorkspace,
    selectedTasks,
    persistError,
    dismissPersistError,
    updateSetup,
    addWorkspace,
    updateWorkspace,
    removeWorkspace,
    selectWorkspace,
    clearWorkspaceSwitching,
    workspaceSwitching,
    addTask,
    removeTask,
    updateEditorApp,
    updateThemeId,
  } = useStore();

  const [showAddWorkspace, setShowAddWorkspace] = useState(false);

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
    p: { handler: () => setShowAddWorkspace(true), enabled: state.setup.isComplete },
  });

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full" data-tauri-drag-region>
        <SpinnerIcon size={24} className="text-text-muted" />
      </div>
    );
  }

  if (!state.setup.isComplete) {
    return <SetupWizard initialSetup={state.setup} onComplete={updateSetup} />;
  }

  return (
    <div className="flex h-full relative">
      {/* Full-width drag region at the very top for window dragging */}
      <div className="absolute top-0 left-0 right-0 h-[32px] z-[5]" data-tauri-drag-region />
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
          showAddExternal={showAddWorkspace}
          onCloseAddExternal={() => setShowAddWorkspace(false)}
          themeId={themeId}
          onThemeChange={updateThemeId}
          defaultLinearApiKey={defaultLinearApiKey}
        />
      </ErrorBoundary>
      <ErrorBoundary fallbackClassName="flex-1">
        <WorktreeList
          tasks={selectedTasks}
          workspace={selectedWorkspace}
          onTaskCreated={addTask}
          onTaskDeleted={removeTask}
          editorApp={editorApp}
          onEditorChange={updateEditorApp}
          workspaceSwitching={workspaceSwitching}
          onWorkspaceReady={clearWorkspaceSwitching}
        />
      </ErrorBoundary>
    </div>
  );
}

export default App;
