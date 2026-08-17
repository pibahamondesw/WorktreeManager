import { load, Store } from "@tauri-apps/plugin-store";
import { AppState, DEFAULT_STATE, EDITOR_APPS, EditorApp, VaultConfig } from "../types";
import {
  migrateLegacyToWorkspaces,
  normalizeSetup,
  normalizeTasks,
  normalizeWorkspaces,
  redactLegacyBackup,
} from "../utils";

let store: Store | null = null;

/**
 * Bump when the persisted shape changes. 1 = multi-repo workspaces/tasks.
 * 2 = global vault config; per-workspace notesPath dropped.
 * 3 = pre-multi-repo keys and the orphaned `setup.githubToken` pruned.
 */
const SCHEMA_VERSION = 3;

/** Pre-multi-repo root keys, superseded by `workspaces`/`tasks`/`selectedWorkspaceId`. */
const LEGACY_KEYS = ["repos", "worktrees", "selectedRepoId"];

const LEGACY_BACKUP_FILE = "store.backup-preMultiRepo.json";

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
 * Write a one-shot, credential-free copy of the legacy store data to a sidecar file so
 * the pre-migration structure can be recovered. Best-effort: never blocks migration.
 */
async function backupLegacyStore(data: Parameters<typeof redactLegacyBackup>[0]): Promise<void> {
  try {
    const backup = await load(LEGACY_BACKUP_FILE, { defaults: {}, autoSave: false });
    if ((await backup.get("repos")) != null) return; // don't clobber an existing backup
    for (const [key, value] of Object.entries(redactLegacyBackup(data))) {
      if (value !== undefined) await backup.set(key, value);
    }
    await backup.set("backedUpAt", new Date().toISOString());
    await backup.save();
  } catch {
    // Backup is best-effort; the user's own manual backup remains.
  }
}

/**
 * Drop the pre-multi-repo keys. Each `repos[].linearApiKey` is a plaintext copy of the
 * Linear token that nothing has read since the workspaces migration. Leaves the store
 * dirty — the caller flushes.
 */
async function pruneLegacyKeys(s: Store): Promise<void> {
  for (const key of LEGACY_KEYS) {
    await s.delete(key);
  }
}

/**
 * Empty the pre-multi-repo backup sidecar written by earlier builds, which held a second
 * plaintext copy of the Linear key alongside the orphaned GitHub token. The store plugin
 * exposes no way to remove the file itself, so an empty `{}` is left behind; a sidecar
 * that never existed is left absent rather than created empty.
 */
async function clearLegacyBackup(): Promise<void> {
  try {
    const backup = await load(LEGACY_BACKUP_FILE, { defaults: {}, autoSave: false });
    if ((await backup.keys()).length === 0) return;
    await backup.clear();
    await backup.save();
  } catch {
    // Nothing recoverable in a sidecar we cannot read.
  }
}

function resolveSelectedWorkspaceId(
  candidate: string | null | undefined,
  workspaces: AppState["workspaces"]
): string | null {
  if (candidate && workspaces.some((w) => w.id === candidate)) return candidate;
  return workspaces[0]?.id ?? null;
}

export async function loadState(): Promise<AppState> {
  const s = await getStore();
  const rawSetup = await s.get<any>("setup");
  const schemaVersion = (await s.get<number>("schemaVersion")) ?? 0;
  const rawWorkspaces = await s.get<any[]>("workspaces");
  const setup = normalizeSetup(rawSetup);

  // Already on the multi-repo schema: load and normalize directly.
  if (schemaVersion >= 1 || rawWorkspaces) {
    const workspaces = normalizeWorkspaces(rawWorkspaces);
    const tasks = normalizeTasks(await s.get<any[]>("tasks"));
    const selectedWorkspaceId = resolveSelectedWorkspaceId(
      await s.get<string | null>("selectedWorkspaceId"),
      workspaces
    );

    // v1 → v2: the per-workspace notesPath is dropped; the global vault starts
    // disabled — enabling is always an explicit user action against the managed
    // path. Keyed on the vault key being absent so a partial write retries.
    let vault = await s.get<VaultConfig>("vault");
    if (vault == null) {
      vault = DEFAULT_STATE.vault;
      await persist([
        ["vault", vault],
        ["workspaces", workspaces],
      ]);
    }

    // v2 → v3: take the stale credentials off disk. `normalizeSetup` already keeps
    // `setup.githubToken` out of memory; rewriting `setup` is what drops it from the
    // file, alongside the legacy keys and the backup sidecar. Keyed on the version so
    // a partial write retries, and ordered last so the bump implies both steps ran.
    if (schemaVersion < 3) {
      await pruneLegacyKeys(s);
      await clearLegacyBackup();
      await persist([
        ["setup", setup],
        ["schemaVersion", SCHEMA_VERSION],
      ]);
    }

    return { setup, vault, workspaces, tasks, selectedWorkspaceId };
  }

  // Legacy single-repo schema: migrate to workspaces/tasks.
  const rawRepos = await s.get<any[]>("repos");
  const rawWorktrees = await s.get<any[]>("worktrees");
  const selectedRepoId = await s.get<string | null>("selectedRepoId");

  const { workspaces, tasks } = migrateLegacyToWorkspaces(
    rawRepos,
    rawWorktrees,
    setup.linearApiKey
  );
  const selectedWorkspaceId = resolveSelectedWorkspaceId(selectedRepoId, workspaces);

  if (rawRepos?.length || rawWorktrees?.length) {
    await backupLegacyStore({ repos: rawRepos, worktrees: rawWorktrees, selectedRepoId, setup });
  }

  // Persist the migrated shape + version. This lands straight on v3, so the legacy keys
  // go now rather than lingering as a rollback point holding a plaintext Linear key —
  // the redacted sidecar above is what structure is recovered from.
  await pruneLegacyKeys(s);
  await persist([
    ["setup", setup],
    ["workspaces", workspaces],
    ["tasks", tasks],
    ["selectedWorkspaceId", selectedWorkspaceId],
    ["vault", DEFAULT_STATE.vault],
    ["schemaVersion", SCHEMA_VERSION],
  ]);

  return {
    setup,
    vault: DEFAULT_STATE.vault,
    workspaces,
    tasks,
    selectedWorkspaceId,
  };
}

export async function loadThemeId(): Promise<string> {
  const s = await getStore();
  return (await s.get<string>("themeId")) ?? "default";
}

/**
 * Load the user's saved custom-theme colors, or `null` when they have never
 * customized. `null` is the signal to seed the custom theme from the active
 * preset the first time it is selected.
 */
export async function loadCustomColors(): Promise<Record<string, string> | null> {
  const s = await getStore();
  return (await s.get<Record<string, string>>("customTheme")) ?? null;
}

export async function loadEditorApp(): Promise<EditorApp> {
  const s = await getStore();
  const stored = await s.get<EditorApp>("editorApp");
  // Guard against stale values persisted by older builds whose editor ids no longer exist.
  return EDITOR_APPS.some((e) => e.id === stored) ? stored! : "cursor";
}
