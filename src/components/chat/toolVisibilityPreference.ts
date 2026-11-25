import { useSyncExternalStore } from "react";
import { TOOL_VISIBILITY_OPTIONS, ToolVisibility } from "./toolVisibility";

const STORAGE_KEY = "worktreemanager.toolVisibility";

const listeners = new Set<() => void>();

const isToolVisibility = (value: string | null): value is ToolVisibility =>
  TOOL_VISIBILITY_OPTIONS.some((option) => option.id === value);

export function readToolVisibility(): ToolVisibility {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return isToolVisibility(stored) ? stored : "all";
  } catch {
    return "all";
  }
}

let current = readToolVisibility();

export function saveToolVisibility(visibility: ToolVisibility) {
  current = visibility;
  try {
    localStorage.setItem(STORAGE_KEY, visibility);
  } catch {
    // Every open chat still switches until the app restarts when storage is unavailable.
  }
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The tool visibility every chat shares, kept across restarts. */
export function useToolVisibility() {
  return useSyncExternalStore(subscribe, () => current);
}
