import { load, Store } from "@tauri-apps/plugin-store";
import { AppState, DEFAULT_STATE } from "../types";
import {
  EMPTY_SECRETS,
  collectSecrets,
  hasSecrets,
  mergeSecrets,
  migrateLegacyToWorkspaces,
  redactLegacyBackup,
  setupWithSecret,
  workspacesWithSecrets,
} from "../utils";
import { loadSecrets, saveAndVerifySecrets } from "./keychain";
import {
  credentialState,
  getStore,
  loadPreference,
  normalizers,
  persist,
  PersistedEntry,
} from "./preferences";
export { persist } from "./preferences";

/**
 * Bump when the persisted shape changes. 1 = multi-repo workspaces/tasks.
 * 2 = global vault config; per-workspace notesPath dropped.
 * 3 = pre-multi-repo keys and the orphaned `setup.githubToken` pruned.
 * 4 = Linear keys moved out of the file and into the keychain.
 */
const SCHEMA_VERSION = 4;

/** Pre-multi-repo root keys, superseded by `workspaces`/`tasks`/`selectedWorkspaceId`. */
const LEGACY_KEYS = ["repos", "worktrees", "selectedRepoId"];

const LEGACY_BACKUP_FILE = "store.backup-preMultiRepo.json";

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

async function persistOrRetryNextLaunch(entries: PersistedEntry[]): Promise<void> {
  await persist(entries).catch(() => {});
}

function resolveSelectedWorkspaceId(
  candidate: string | null | undefined,
  workspaces: AppState["workspaces"]
): string | null {
  if (candidate && workspaces.some((w) => w.id === candidate)) return candidate;
  return workspaces[0]?.id ?? null;
}

export async function restoreKeychainSecrets(getState: () => AppState): Promise<AppState> {
  const stored = await loadSecrets();
  if (!stored.secrets) throw new Error(stored.error);
  const state = getState();
  credentialState.secrets = mergeSecrets(
    stored.secrets,
    collectSecrets(state.setup, state.workspaces)
  );
  credentialState.readable = true;
  return {
    ...state,
    setup: setupWithSecret(state.setup, credentialState.secrets),
    workspaces: workspacesWithSecrets(state.workspaces, credentialState.secrets),
  };
}

export async function loadState(
  onKeychainError?: (error: string | null) => void
): Promise<AppState> {
  const s = await getStore();
  const shortcutOverrides = await loadPreference("shortcutOverrides");
  const schemaVersion = await loadPreference("schemaVersion");
  const rawWorkspaces = await s.get<unknown[]>("workspaces");
  const setup = await loadPreference("setup");

  // Already on the multi-repo schema: load and normalize directly.
  if (schemaVersion >= 1 || rawWorkspaces) {
    const workspaces = normalizers.workspaces(rawWorkspaces);
    const tasks = await loadPreference("tasks");
    const selectedWorkspaceId = resolveSelectedWorkspaceId(
      await loadPreference("selectedWorkspaceId"),
      workspaces
    );

    // Linear keys live in the keychain from v4 on. Both sources are read and merged with
    // the file winning, so a store caught between the two — keys already in the keychain
    // but the version not yet bumped, or the reverse — still comes up holding all of them.
    // Loaded before the migrations below because each one persists through `persist`.
    const stored = await loadSecrets();
    onKeychainError?.(stored.error);
    credentialState.readable = stored.secrets != null;
    credentialState.secrets = mergeSecrets(
      stored.secrets ?? EMPTY_SECRETS,
      collectSecrets(setup, workspaces)
    );

    // v1 → v2: the per-workspace notesPath is dropped; the global vault starts
    // disabled — enabling is always an explicit user action against the managed
    // path. Keyed on the vault key being absent so a partial write retries.
    const rawVault = await s.get<unknown>("vault");
    const vault = normalizers.vault(rawVault);
    if (rawVault == null) {
      await persistOrRetryNextLaunch([
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
      await persistOrRetryNextLaunch([
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
      if (
        !hasSecrets(credentialState.secrets) ||
        (credentialState.readable && (await saveAndVerifySecrets(credentialState.secrets)))
      ) {
        await persistOrRetryNextLaunch([
          ["setup", setup],
          ["workspaces", workspaces],
          ["schemaVersion", SCHEMA_VERSION],
        ]);
      }
    }

    return {
      setup: setupWithSecret(setup, credentialState.secrets),
      shortcutOverrides,
      vault,
      workspaces: workspacesWithSecrets(workspaces, credentialState.secrets),
      tasks,
      selectedWorkspaceId,
    };
  }

  // Legacy single-repo schema: migrate to workspaces/tasks.
  const rawRepos = await s.get<unknown[]>("repos");
  const rawWorktrees = await s.get<unknown[]>("worktrees");
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
  credentialState.secrets = collectSecrets(setup, workspaces);
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
    shortcutOverrides,
    vault: DEFAULT_STATE.vault,
    workspaces,
    tasks,
    selectedWorkspaceId,
  };
}

export const loadThemeId = () => loadPreference("themeId");
export const loadCustomColors = () => loadPreference("customTheme");
export const loadEditorApp = () => loadPreference("editorApp");
export const loadAgentViews = () => loadPreference("agentViews");
export const loadScriptApprovals = () => loadPreference("scriptApprovals");
export const loadSidebarCollapsed = () => loadPreference("sidebarCollapsed");
export const loadAgentAlerts = () => loadPreference("agentAlerts");
export const loadNavigationHistory = () => loadPreference("navigationHistory");
