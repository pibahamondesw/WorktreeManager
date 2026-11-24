import type { DiffLayout } from "./FileDiffView";

const STORAGE_KEY = "worktreemanager.diffLayout";

export function readDiffLayout(): DiffLayout {
  try {
    return localStorage.getItem(STORAGE_KEY) === "split" ? "split" : "unified";
  } catch {
    return "unified";
  }
}

export function saveDiffLayout(layout: DiffLayout) {
  try {
    localStorage.setItem(STORAGE_KEY, layout);
  } catch {
    // The layout still switches for this panel when storage is unavailable.
  }
}
