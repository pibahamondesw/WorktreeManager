import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  AutomationRun,
  AutomationSummary,
  RunItem,
  listAutomationItems,
  listAutomationRuns,
  loadAutomationSummaries,
} from "../services/automations";

const RUNS_PAGE_SIZE = 50;
/** Emitted by the engine when a run ends or records a request the Inbox shows. */
const RUNS_CHANGED_EVENT = "automation-runs-changed";
/** A burst of changes reloads once. */
const RUNS_CHANGED_DEBOUNCE_MS = 250;

/** Loads now and again whenever the window regains focus, so edits made elsewhere show up. */
function useLoadOnFocus(load: () => void) {
  useEffect(() => {
    load();
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, [load]);
}

function useReloadOnRunsChanged(load: () => void) {
  useEffect(() => {
    let timer: number | undefined;
    const unlisten = listen<string>(RUNS_CHANGED_EVENT, () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(load, RUNS_CHANGED_DEBOUNCE_MS);
    });
    return () => {
      window.clearTimeout(timer);
      void unlisten.then((stop) => stop());
    };
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
  useReloadOnRunsChanged(reload);
  return { automations, error, reload };
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
    reload,
    loadMore: current.nextBefore ? () => load(current.nextBefore) : null,
  };
}

interface ItemsState {
  name: string;
  items: RunItem[];
  error: string | null;
  loading: boolean;
}

/** The campaign's board items, reloaded on focus and whenever the engine records a change. */
export function useAutomationItems(name: string) {
  const [page, setPage] = useState<ItemsState>({ name, items: [], error: null, loading: true });
  const startRequest = useLatestRequest();

  const reload = useCallback(() => {
    const isLatest = startRequest();
    listAutomationItems(name)
      .then((items) => {
        if (isLatest()) setPage({ name, items, error: null, loading: false });
      })
      .catch((reason: unknown) => {
        if (isLatest())
          setPage((current) => ({ ...current, name, error: String(reason), loading: false }));
      });
  }, [name, startRequest]);

  useLoadOnFocus(reload);
  useReloadOnRunsChanged(reload);
  const current = page.name === name ? page : { name, items: [], error: null, loading: true };
  return { items: current.items, error: current.error, loading: current.loading, reload };
}
