import { load, Store } from "@tauri-apps/plugin-store";
import { AppState, DEFAULT_STATE, EDITOR_APPS, EditorApp, VaultConfig, Workspace } from "../types";
import {
  EMPTY_SECRETS,
  SecretBundle,
  collectSecrets,
  hasSecrets,
  mergeSecrets,
  migrateLegacyToWorkspaces,
  normalizeSetup,
  normalizeTasks,
  normalizeWorkspaces,
  redactLegacyBackup,
  setupWithSecret,
  setupWithoutSecret,
  workspacesWithSecrets,
  workspacesWithoutSecrets,
} from "../utils";
import { loadSecrets, saveAndVerifySecrets, saveSecrets } from "./keychain";

let store: Store | null = null;

/**
 * Bump when the persisted shape changes. 1 = multi-repo workspaces/tasks.
 * 2 = global vault config; per-workspace notesPath dropped.
 * 3 = pre-multi-repo keys and the orphaned `setup.githubToken` pruned.
 * 4 = Linear keys moved out of the file and into the keychain.
 */
const SCHEMA_VERSION = 4;

/**
 * The Linear keys as last written to the keychain. `persist` keeps this current so a
 * write that touches only `workspaces` does not drop the setup key from the blob.
 */
let secrets: SecretBundle = EMPTY_SECRETS;

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
 *
 * The Linear keys are diverted to the keychain on the way through, so callers can keep
 * passing whole `setup`/`workspaces` values without knowing where the secrets end up.
 */
export async function persist(entries: [string, unknown][]): Promise<void> {
  const s = await getStore();
  let next = secrets;
  let carriesSecrets = false;
  const toWrite: [string, unknown][] = [];

  for (const [key, value] of entries) {
    if (key === "setup") {
      const setup = value as AppState["setup"];
      next = { ...next, setup: setup.linearApiKey ?? null };
      carriesSecrets = true;
      toWrite.push([key, setupWithoutSecret(setup)]);
    } else if (key === "workspaces") {
      const workspaces = value as Workspace[];
      next = { ...next, workspaces: collectSecrets(null, workspaces).workspaces };
      carriesSecrets = true;
      toWrite.push([key, workspacesWithoutSecrets(workspaces)]);
    } else {
      toWrite.push([key, value]);
    }
  }

  // The keychain is written first and its failure aborts the whole write: taking a key
  // out of the file is only safe once the keychain holds it. The caller rolls its state
  // back and the file still agrees with what the keychain has.
  if (carriesSecrets) {
    await saveSecrets(next);
    secrets = next;
  }

  for (const [key, value] of toWrite) {
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

    // Linear keys live in the keychain from v4 on. Both sources are read and merged with
    // the file winning, so a store caught between the two — keys already in the keychain
    // but the version not yet bumped, or the reverse — still comes up holding all of them.
    // Loaded before the migrations below because each one persists through `persist`.
    secrets = mergeSecrets(
      (await loadSecrets()) ?? EMPTY_SECRETS,
      collectSecrets(setup, workspaces)
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
        // Records v3 rather than SCHEMA_VERSION: the v4 step owns its own bump, so a
        // keychain that cannot be reached can never leave the version overstated.
        ["schemaVersion", 3],
      ]);
    }

    // v3 → v4: lift the Linear keys off disk. They are written and read back before the
    // file is rewritten, so it is only stripped once the keychain demonstrably holds
    // them; an unreachable keychain leaves the file as it is and retries next launch.
    if (schemaVersion < 4) {
      try {
        if (!hasSecrets(secrets) || (await saveAndVerifySecrets(secrets))) {
          await persist([
            ["setup", setup],
            ["workspaces", workspaces],
            ["schemaVersion", SCHEMA_VERSION],
          ]);
        }
      } catch {
        // Keychain unavailable. The keys stay in the file and the version stays put,
        // so the app keeps working and the move is retried on the next launch.
      }
    }

    return {
      setup: setupWithSecret(setup, secrets),
      vault,
      workspaces: workspacesWithSecrets(workspaces, secrets),
      tasks,
      selectedWorkspaceId,
    };
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

  // Persist the migrated shape + version. This lands straight on the current version, so
  // the legacy keys go now rather than lingering as a rollback point holding a plaintext
  // Linear key — the redacted sidecar above is what structure is recovered from. The keys
  // reach the keychain through `persist`, whose failure aborts before the file is written.
  await pruneLegacyKeys(s);
  secrets = collectSecrets(setup, workspaces);
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
