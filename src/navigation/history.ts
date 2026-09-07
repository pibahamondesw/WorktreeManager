export type NavigationEntry =
  | { kind: "workspace"; workspaceId: string; at: string }
  | { kind: "task"; taskId: string; workspaceId: string; at: string };

export interface NavigationHistory {
  entries: NavigationEntry[];
  cursor: number;
}

export type NavigationDirection = -1 | 1;

export const MAX_HISTORY_ENTRIES = 200;

export const EMPTY_HISTORY: NavigationHistory = { entries: [], cursor: -1 };

export function historyFromEntries(entries: NavigationEntry[]): NavigationHistory {
  return { entries, cursor: entries.length - 1 };
}

export function sameTarget(a: NavigationEntry, b: NavigationEntry): boolean {
  if (a.kind === "workspace" && b.kind === "workspace") return a.workspaceId === b.workspaceId;
  if (a.kind === "task" && b.kind === "task") return a.taskId === b.taskId;
  return false;
}

/**
 * Browser-style push onto the session stack: entries ahead of the cursor are dropped and the
 * cursor moves to the new entry. Revisiting the current target only refreshes its timestamp.
 */
export function pushEntry(history: NavigationHistory, entry: NavigationEntry): NavigationHistory {
  const current = history.entries[history.cursor];
  if (current && sameTarget(current, entry)) {
    const entries = [...history.entries];
    entries[history.cursor] = entry;
    return { entries, cursor: history.cursor };
  }
  const entries = [...history.entries.slice(0, history.cursor + 1), entry].slice(
    -MAX_HISTORY_ENTRIES
  );
  return { entries, cursor: entries.length - 1 };
}

/** Append to the persisted visit log (recency source for search); never drops earlier visits. */
export function appendVisit(log: NavigationEntry[], entry: NavigationEntry): NavigationEntry[] {
  const last = log[log.length - 1];
  const base = last && sameTarget(last, entry) ? log.slice(0, -1) : log;
  return [...base, entry].slice(-MAX_HISTORY_ENTRIES);
}

/** A new session's stack starts at the last visit of the previous one, then replays `recorded`. */
export function seedSession(
  previous: NavigationEntry[],
  recorded: NavigationEntry[]
): NavigationHistory {
  return recorded.reduce(pushEntry, historyFromEntries(previous.slice(-1)));
}

/**
 * Index of the nearest entry in `direction` that still resolves to something visitable,
 * or `null` when there is none. Stale entries (deleted tasks or workspaces) are skipped.
 */
export function nextNavigableIndex(
  history: NavigationHistory,
  direction: NavigationDirection,
  isNavigable: (entry: NavigationEntry) => boolean
): number | null {
  for (let i = history.cursor + direction; i >= 0 && i < history.entries.length; i += direction) {
    if (isNavigable(history.entries[i])) return i;
  }
  return null;
}

export function isNavigationEntry(value: unknown): value is NavigationEntry {
  if (!value || typeof value !== "object") return false;
  const e = value as Record<string, unknown>;
  if (typeof e.workspaceId !== "string" || typeof e.at !== "string") return false;
  if (e.kind === "workspace") return true;
  return e.kind === "task" && typeof e.taskId === "string";
}
