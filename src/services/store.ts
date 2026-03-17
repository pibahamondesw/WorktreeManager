import { load, Store } from "@tauri-apps/plugin-store";
import { AppState, DEFAULT_STATE, EditorApp, Worktree } from "../types";
import { migrateRepos } from "../utils";

let store: Store | null = null;

async function getStore(): Promise<Store> {
  if (!store) {
    store = await load("store.json", { defaults: {}, autoSave: false });
  }
  return store;
}

/**
 * Set one or more keys in the store and flush to disk in a single write.
 * All keys are updated in memory first, then persisted atomically.
 */
export async function persist(entries: [string, unknown][]): Promise<void> {
  const s = await getStore();
  for (const [key, value] of entries) {
    await s.set(key, value);
  }
  await s.save();
}

export async function loadState(): Promise<AppState> {
  const s = await getStore();
  const setup = await s.get<AppState["setup"]>("setup");
  const rawRepos = await s.get<any[]>("repos");
  const worktrees = await s.get<Worktree[]>("worktrees");
  const selectedRepoId = await s.get<string | null>("selectedRepoId");

  const repos = migrateRepos(rawRepos ?? [], setup?.linearApiKey);

  // Persist migrated repos if the global key was copied to repos
  if (setup?.linearApiKey && rawRepos?.length && rawRepos.every((r: any) => !r.linearApiKey)) {
    await s.set("repos", repos);
    await s.save();
  }

  return {
    setup: setup ?? DEFAULT_STATE.setup,
    repos,
    worktrees: worktrees ?? DEFAULT_STATE.worktrees,
    selectedRepoId: selectedRepoId ?? DEFAULT_STATE.selectedRepoId,
  };
}

export async function loadThemeId(): Promise<string> {
  const s = await getStore();
  return (await s.get<string>("themeId")) ?? "default";
}

export async function loadEditorApp(): Promise<EditorApp> {
  const s = await getStore();
  return (await s.get<EditorApp>("editorApp")) ?? "cursor";
}
