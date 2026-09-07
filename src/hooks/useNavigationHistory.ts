import { useCallback, useEffect, useRef, useState } from "react";
import { loadNavigationHistory, persist } from "../services/store";
import {
  EMPTY_HISTORY,
  NavigationDirection,
  NavigationEntry,
  NavigationHistory,
  appendVisit,
  nextNavigableIndex,
  pushEntry,
  seedSession,
} from "../navigation/history";
import { Task } from "../types";

interface Params {
  /** Whether the entry still points at something that exists (workspaces and tasks get deleted). */
  isNavigable: (entry: NavigationEntry) => boolean;
  /** Show the entry without recording it again. */
  onNavigate: (entry: NavigationEntry) => void;
}

/**
 * Browser-style back/forward over the places the user visited: workspaces selected and tasks
 * opened or revealed. The stack is per session, seeded with the last visit of the previous one.
 * Every visit is also appended to a persisted log (`navigationHistory`) meant to feed recency
 * into search.
 */
export function useNavigationHistory({ isNavigable, onNavigate }: Params) {
  const [history, setHistory] = useState<NavigationHistory>(EMPTY_HISTORY);
  const historyRef = useRef(history);
  const logRef = useRef<NavigationEntry[]>([]);

  const commit = useCallback((next: NavigationHistory) => {
    historyRef.current = next;
    setHistory(next);
  }, []);

  useEffect(() => {
    let cancelled = false;
    loadNavigationHistory()
      .then((loaded) => {
        if (cancelled) return;
        const recorded = historyRef.current.entries;
        logRef.current = recorded.reduce(appendVisit, loaded);
        commit(seedSession(loaded, recorded));
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [commit]);

  const record = useCallback(
    (entry: NavigationEntry) => {
      commit(pushEntry(historyRef.current, entry));
      logRef.current = appendVisit(logRef.current, entry);
      persist([["navigationHistory", logRef.current]]).catch(() => undefined);
    },
    [commit]
  );

  const recordWorkspaceVisit = useCallback(
    (workspaceId: string) =>
      record({ kind: "workspace", workspaceId, at: new Date().toISOString() }),
    [record]
  );

  const recordTaskVisit = useCallback(
    (task: Task) =>
      record({
        kind: "task",
        taskId: task.id,
        workspaceId: task.workspaceId,
        at: new Date().toISOString(),
      }),
    [record]
  );

  const step = useCallback(
    (direction: NavigationDirection) => {
      const current = historyRef.current;
      const index = nextNavigableIndex(current, direction, isNavigable);
      if (index === null) return;
      commit({ ...current, cursor: index });
      onNavigate(current.entries[index]);
    },
    [commit, isNavigable, onNavigate]
  );

  return {
    entries: history.entries,
    canGoBack: nextNavigableIndex(history, -1, isNavigable) !== null,
    canGoForward: nextNavigableIndex(history, 1, isNavigable) !== null,
    back: useCallback(() => step(-1), [step]),
    forward: useCallback(() => step(1), [step]),
    recordWorkspaceVisit,
    recordTaskVisit,
  };
}
