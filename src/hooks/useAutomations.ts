import { useCallback, useEffect, useRef, useState } from "react";
import {
  AutomationRun,
  AutomationSummary,
  listAutomationRuns,
  loadAutomationSummaries,
} from "../services/automations";

const RUNS_PAGE_SIZE = 50;

/** Loads now and again whenever the window regains focus, so edits made elsewhere show up. */
function useLoadOnFocus(load: () => void) {
  useEffect(() => {
    load();
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, [load]);
}

/** Responses of superseded requests are dropped, so a slow load never overwrites a newer one. */
function useLatestRequest() {
  const latest = useRef(0);
  return useCallback(() => {
    const id = ++latest.current;
    return () => id === latest.current;
  }, []);
}

export function useAutomations() {
  const [automations, setAutomations] = useState<AutomationSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const startRequest = useLatestRequest();

  const reload = useCallback(() => {
    const isLatest = startRequest();
    loadAutomationSummaries()
      .then((summaries) => {
        if (!isLatest()) return;
        setAutomations(summaries);
        setError(null);
      })
      .catch((reason: unknown) => {
        if (isLatest()) setError(String(reason));
      });
  }, [startRequest]);

  useLoadOnFocus(reload);
  return { automations, error };
}

interface RunsState {
  name: string;
  runs: AutomationRun[];
  nextBefore: string | null;
  error: string | null;
  loading: boolean;
}

export function useAutomationRuns(name: string) {
  const [page, setPage] = useState<RunsState>({
    name,
    runs: [],
    nextBefore: null,
    error: null,
    loading: true,
  });
  const startRequest = useLatestRequest();

  const load = useCallback(
    (before: string | null) => {
      const isLatest = startRequest();
      setPage((current) =>
        current.name === name
          ? { ...current, loading: true }
          : { name, runs: [], nextBefore: null, error: null, loading: true }
      );
      listAutomationRuns(name, before, RUNS_PAGE_SIZE)
        .then(({ runs, next_before }) => {
          if (!isLatest()) return;
          setPage((current) => ({
            name,
            runs: before ? [...current.runs, ...runs] : runs,
            nextBefore: next_before,
            error: null,
            loading: false,
          }));
        })
        .catch((reason: unknown) => {
          if (isLatest())
            setPage((current) => ({ ...current, error: String(reason), loading: false }));
        });
    },
    [name, startRequest]
  );

  const reload = useCallback(() => load(null), [load]);
  useLoadOnFocus(reload);

  const current: RunsState =
    page.name === name ? page : { name, runs: [], nextBefore: null, error: null, loading: true };
  return {
    runs: current.runs,
    error: current.error,
    loading: current.loading,
    loadMore: current.nextBefore ? () => load(current.nextBefore) : null,
  };
}
