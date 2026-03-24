import { useState, useEffect, useCallback, useMemo } from "react";
import { loadState, loadEditorApp, loadThemeId, persist } from "../services/store";
import { AppState, DEFAULT_STATE, EditorApp, Repo, Worktree } from "../types";
import { applyTheme } from "../themes";

export function useStore() {
  const [state, setState] = useState<AppState>(DEFAULT_STATE);
  const [loading, setLoading] = useState(true);
  const [editorApp, setEditorAppState] = useState<EditorApp>("cursor");
  const [themeId, setThemeIdState] = useState("default");
  const [repoSwitching, setRepoSwitching] = useState(true);
  const [persistError, setPersistError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([loadState(), loadEditorApp(), loadThemeId()]).then(([s, editor, theme]) => {
      setState(s);
      setEditorAppState(editor);
      setThemeIdState(theme);
      applyTheme(theme);
      setLoading(false);
    });
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

  const addRepo = useCallback(async (repo: Repo) => {
    let snapshot: AppState;
    let newRepos: Repo[];
    let shouldSelectNew = false;
    setState((prev) => {
      snapshot = prev;
      newRepos = [...prev.repos, repo];
      shouldSelectNew = prev.selectedRepoId === null;
      return {
        ...prev,
        repos: newRepos,
        selectedRepoId: prev.selectedRepoId ?? repo.id,
      };
    });
    const entries: [string, unknown][] = [["repos", newRepos!]];
    if (shouldSelectNew) entries.push(["selectedRepoId", repo.id]);
    try {
      await persist(entries);
    } catch {
      setState(snapshot!);
      setPersistError("Failed to save project");
    }
  }, []);

  const updateRepo = useCallback(async (repoId: string, updates: Partial<Pick<Repo, "name" | "linearApiKey">>) => {
    let snapshot: AppState;
    let newRepos: Repo[];
    setState((prev) => {
      snapshot = prev;
      newRepos = prev.repos.map((r) => (r.id === repoId ? { ...r, ...updates } : r));
      return { ...prev, repos: newRepos };
    });
    try {
      await persist([["repos", newRepos!]]);
    } catch {
      setState(snapshot!);
      setPersistError("Failed to save project changes");
    }
  }, []);

  const removeRepo = useCallback(async (repoId: string) => {
    let snapshot: AppState;
    let newRepos: Repo[];
    let newWorktrees: Worktree[];
    let newSelectedId: string | null;
    setState((prev) => {
      snapshot = prev;
      newRepos = prev.repos.filter((r) => r.id !== repoId);
      newWorktrees = prev.worktrees.filter((w) => w.repoId !== repoId);
      newSelectedId =
        prev.selectedRepoId === repoId ? (newRepos[0]?.id ?? null) : prev.selectedRepoId;
      return {
        ...prev,
        repos: newRepos,
        worktrees: newWorktrees,
        selectedRepoId: newSelectedId,
      };
    });
    try {
      await persist([
        ["repos", newRepos!],
        ["worktrees", newWorktrees!],
        ["selectedRepoId", newSelectedId!],
      ]);
    } catch {
      setState(snapshot!);
      setPersistError("Failed to save project removal");
    }
  }, []);

  const selectRepo = useCallback((repoId: string) => {
    let snapshot: AppState;
    setState((prev) => {
      snapshot = prev;
      return { ...prev, selectedRepoId: repoId };
    });
    setRepoSwitching(true);
    setTimeout(async () => {
      try {
        await persist([["selectedRepoId", repoId]]);
      } catch {
        setState(snapshot!);
        setPersistError("Failed to save selection");
      }
    }, 0);
  }, []);

  const clearRepoSwitching = useCallback(() => setRepoSwitching(false), []);

  const addWorktree = useCallback(async (worktree: Worktree) => {
    let snapshot: AppState;
    let newWorktrees: Worktree[];
    setState((prev) => {
      snapshot = prev;
      newWorktrees = [...prev.worktrees, worktree];
      return { ...prev, worktrees: newWorktrees };
    });
    try {
      await persist([["worktrees", newWorktrees!]]);
    } catch {
      setState(snapshot!);
      setPersistError("Failed to save worktree");
    }
  }, []);

  const removeWorktree = useCallback(async (worktreeId: string) => {
    let snapshot: AppState;
    let newWorktrees: Worktree[];
    setState((prev) => {
      snapshot = prev;
      newWorktrees = prev.worktrees.filter((w) => w.id !== worktreeId);
      return { ...prev, worktrees: newWorktrees };
    });
    try {
      await persist([["worktrees", newWorktrees!]]);
    } catch {
      setState(snapshot!);
      setPersistError("Failed to save worktree removal");
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

  const updateThemeId = useCallback(async (id: string) => {
    let prevTheme: string;
    setThemeIdState((current) => {
      prevTheme = current;
      return id;
    });
    applyTheme(id);
    try {
      await persist([["themeId", id]]);
    } catch {
      setThemeIdState(prevTheme!);
      applyTheme(prevTheme!);
      setPersistError("Failed to save theme preference");
    }
  }, []);

  const selectedRepo = useMemo(
    () => state.repos.find((r) => r.id === state.selectedRepoId),
    [state.repos, state.selectedRepoId]
  );
  const selectedWorktrees = useMemo(
    () => state.worktrees.filter((w) => w.repoId === state.selectedRepoId),
    [state.worktrees, state.selectedRepoId]
  );

  return {
    state,
    loading,
    editorApp,
    themeId,
    selectedRepo,
    selectedWorktrees,
    repoSwitching,
    persistError,
    dismissPersistError,
    updateSetup,
    addRepo,
    updateRepo,
    removeRepo,
    selectRepo,
    clearRepoSwitching,
    addWorktree,
    removeWorktree,
    updateEditorApp,
    updateThemeId,
  };
}
