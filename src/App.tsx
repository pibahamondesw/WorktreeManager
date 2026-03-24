import { useState, useMemo, useCallback } from "react";
import { SetupWizard } from "./components/setup/SetupWizard";
import { RepoList } from "./components/sidebar/RepoList";
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
    selectedRepo,
    selectedWorktrees,
    persistError,
    dismissPersistError,
    updateSetup,
    addRepo,
    updateRepo,
    removeRepo,
    selectRepo,
    clearRepoSwitching,
    repoSwitching,
    addWorktree,
    removeWorktree,
    updateEditorApp,
    updateThemeId,
  } = useStore();

  const [showAddProject, setShowAddProject] = useState(false);

  const handleSelectRepo = useCallback(
    (repoId: string) => {
      if (repoId === state.selectedRepoId) return;
      selectRepo(repoId);
    },
    [state.selectedRepoId, selectRepo]
  );

  const defaultLinearApiKey = useMemo(() => {
    const lastRepoWithKey = [...state.repos].reverse().find((r) => r.linearApiKey);
    return lastRepoWithKey?.linearApiKey ?? state.setup.linearApiKey ?? null;
  }, [state.repos, state.setup.linearApiKey]);

  useKeyboardShortcuts({
    p: { handler: () => setShowAddProject(true), enabled: state.setup.isComplete },
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
        <RepoList
          repos={state.repos}
          worktrees={state.worktrees}
          selectedRepoId={state.selectedRepoId}
          onSelect={handleSelectRepo}
          onAdd={addRepo}
          onUpdate={updateRepo}
          onRemove={removeRepo}
          showAddExternal={showAddProject}
          onCloseAddExternal={() => setShowAddProject(false)}
          themeId={themeId}
          onThemeChange={updateThemeId}
          defaultLinearApiKey={defaultLinearApiKey}
        />
      </ErrorBoundary>
      <ErrorBoundary fallbackClassName="flex-1">
        <WorktreeList
          worktrees={selectedWorktrees}
          repo={selectedRepo}
          onWorktreeCreated={addWorktree}
          onWorktreeDeleted={removeWorktree}
          editorApp={editorApp}
          onEditorChange={updateEditorApp}
          repoSwitching={repoSwitching}
          onRepoReady={clearRepoSwitching}
        />
      </ErrorBoundary>
    </div>
  );
}

export default App;
