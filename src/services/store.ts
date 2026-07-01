import { load, Store } from "@tauri-apps/plugin-store";
import { AppState, DEFAULT_STATE, EDITOR_APPS, EditorApp } from "../types";
import { migrateLegacyToWorkspaces, normalizeTasks, normalizeWorkspaces } from "../utils";

let store: Store | null = null;

/** Bump when the persisted shape changes. 1 = multi-repo workspaces/tasks. */
const SCHEMA_VERSION = 1;

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

/**
 * Write a one-shot copy of the legacy store data to a sidecar file so the pre-migration
 * state can be recovered. Best-effort: never blocks migration. The legacy keys are also
 * left untouched in the main store as an additional in-place rollback point.
 */
async function backupLegacyStore(data: Record<string, unknown>): Promise<void> {
  try {
    const backup = await load("store.backup-preMultiRepo.json", { defaults: {}, autoSave: false });
    if ((await backup.get("repos")) != null) return; // don't clobber an existing backup
    for (const [key, value] of Object.entries(data)) {
      if (value !== undefined) await backup.set(key, value);
    }
    await backup.set("backedUpAt", new Date().toISOString());
    await backup.save();
  } catch {
    // Backup is best-effort; the user's manual backup + untouched legacy keys remain.
  }
}

function resolveSelectedWorkspaceId(
  candidate: string | null | undefined,
  workspaces: AppState["workspaces"],
): string | null {
  if (candidate && workspaces.some((w) => w.id === candidate)) return candidate;
  return workspaces[0]?.id ?? null;
}

export async function loadState(): Promise<AppState> {
  const s = await getStore();
  const setup = await s.get<AppState["setup"]>("setup");
  const schemaVersion = (await s.get<number>("schemaVersion")) ?? 0;
  const rawWorkspaces = await s.get<any[]>("workspaces");

  // Already on the multi-repo schema: load and normalize directly.
  if (schemaVersion >= 1 || rawWorkspaces) {
    const workspaces = normalizeWorkspaces(rawWorkspaces);
    const tasks = normalizeTasks(await s.get<any[]>("tasks"));
    const selectedWorkspaceId = resolveSelectedWorkspaceId(
      await s.get<string | null>("selectedWorkspaceId"),
      workspaces,
    );
    return { setup: setup ?? DEFAULT_STATE.setup, workspaces, tasks, selectedWorkspaceId };
  }

  // Legacy single-repo schema: migrate to workspaces/tasks.
  const rawRepos = await s.get<any[]>("repos");
  const rawWorktrees = await s.get<any[]>("worktrees");
  const selectedRepoId = await s.get<string | null>("selectedRepoId");

  const { workspaces, tasks } = migrateLegacyToWorkspaces(
    rawRepos,
    rawWorktrees,
    setup?.linearApiKey,
  );
  const selectedWorkspaceId = resolveSelectedWorkspaceId(selectedRepoId, workspaces);

  if (rawRepos?.length || rawWorktrees?.length) {
    await backupLegacyStore({ repos: rawRepos, worktrees: rawWorktrees, selectedRepoId, setup });
  }

  // Persist the migrated shape + version. Legacy keys are intentionally left in place.
  await persist([
    ["workspaces", workspaces],
    ["tasks", tasks],
    ["selectedWorkspaceId", selectedWorkspaceId],
    ["schemaVersion", SCHEMA_VERSION],
  ]);

  return { setup: setup ?? DEFAULT_STATE.setup, workspaces, tasks, selectedWorkspaceId };
}

export async function loadThemeId(): Promise<string> {
  const s = await getStore();
  return (await s.get<string>("themeId")) ?? "default";
}

export async function loadEditorApp(): Promise<EditorApp> {
  const s = await getStore();
  const stored = await s.get<EditorApp>("editorApp");
  // Guard against stale values persisted by older builds whose editor ids no longer exist.
  return EDITOR_APPS.some((e) => e.id === stored) ? stored! : "cursor";
}
