import { AppState, RepoScripts, Task, Workspace } from "./types";

export function timeAgo(epoch: number): { label: string; stale: boolean; veryStale: boolean } {
  if (!epoch) return { label: "", stale: false, veryStale: false };
  const now = Date.now() / 1000;
  const diff = now - epoch;
  const days = Math.floor(diff / 86400);
  const hours = Math.floor(diff / 3600);
  const minutes = Math.floor(diff / 60);

  let label: string;
  if (days > 30) label = `${Math.floor(days / 30)}mo ago`;
  else if (days > 0) label = `${days}d ago`;
  else if (hours > 0) label = `${hours}h ago`;
  else if (minutes > 0) label = `${minutes}m ago`;
  else label = "just now";

  return { label, stale: days >= 3 && days < 7, veryStale: days >= 7 };
}

export function linearIssueUrl(identifier: string, orgUrlKey?: string | null): string {
  return orgUrlKey
    ? `https://linear.app/${orgUrlKey}/issue/${identifier}`
    : `https://linear.app/issue/${identifier}`;
}

/** "owner/repo" from a GitHub remote URL, lowercased; null for non-GitHub remotes. */
export function githubSlugFromRemote(remoteUrl: string): string | null {
  const match = remoteUrl.match(/github\.com[/:]([^/]+\/[^/]+?)(?:\.git)?$/);
  return match ? match[1].toLowerCase() : null;
}

function persistedRecord(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("Invalid persisted record");
  }
  return value as Record<string, unknown>;
}

function persistedArray(value: unknown): unknown[] {
  if (value == null) return [];
  if (!Array.isArray(value)) throw new Error("Invalid persisted array");
  return value;
}

function persistedString(value: unknown): string | undefined {
  if (value == null) return undefined;
  if (typeof value !== "string") throw new Error("Invalid persisted string");
  return value;
}

function persistedId(value: unknown): string {
  const id = persistedString(value);
  if (!id) throw new Error("Missing persisted ID");
  return id;
}

function persistedBoolean(value: unknown): boolean {
  if (value == null) return false;
  if (typeof value !== "boolean") throw new Error("Invalid persisted boolean");
  return value;
}

/** Keep only non-blank phases; undefined when no phase is overridden. */
export function normalizeRepoScripts(raw: unknown): RepoScripts | undefined {
  if (raw == null) return undefined;
  const record = persistedRecord(raw);
  const scripts: RepoScripts = {};
  for (const phase of ["setup", "teardown"] as const) {
    const script = persistedString(record[phase])?.trim();
    if (script) scripts[phase] = script;
  }
  return Object.keys(scripts).length ? scripts : undefined;
}

/**
 * Normalize workspace objects loaded from the store, filling defaults for fields
 * that may be missing in data written by older builds.
 */
export function normalizeWorkspaces(raw: unknown): Workspace[] {
  return persistedArray(raw).map((value) => {
    const w = persistedRecord(value);
    return {
      id: persistedId(w.id),
      name: persistedString(w.name) ?? "",
      linearApiKey: persistedString(w.linearApiKey) ?? null,
      linearOrgUrlKey: persistedString(w.linearOrgUrlKey) ?? null,
      repos: persistedArray(w.repos).map((value) => {
        const r = persistedRecord(value);
        const scripts = normalizeRepoScripts(r.scripts);
        return {
          id: persistedId(r.id),
          name: persistedString(r.name) ?? "",
          localPath: persistedString(r.localPath) ?? "",
          worktreeBasePath: persistedString(r.worktreeBasePath) ?? "",
          ...(scripts ? { scripts } : {}),
        };
      }),
    };
  });
}

/**
 * Reduce the persisted `setup` blob to the fields the app still uses. The store
 * hands back whatever it finds on disk, so stale keys survive unless dropped
 * explicitly — notably `githubToken`, written by builds from before PR info moved
 * to Linear attachments and never read since.
 */
export function normalizeSetup(raw: unknown): AppState["setup"] {
  const setup = persistedRecord(raw ?? {});
  return {
    linearApiKey: persistedString(setup.linearApiKey) ?? null,
    isComplete: persistedBoolean(setup.isComplete),
  };
}

/**
 * Legacy `repos[]` fields worth keeping in the backup. An allowlist rather than a
 * denylist so a field added by some older build can never leak through.
 */
const LEGACY_REPO_FIELDS = ["id", "name", "localPath", "worktreeBasePath"] as const;

/**
 * Strip credentials out of pre-multi-repo store data before it is written to the
 * rollback backup. The backup exists to recover repo/worktree structure; anyone
 * restoring from it re-enters their Linear key, so a second plaintext copy of the
 * token is liability with no upside.
 */
export function redactLegacyBackup(data: {
  repos?: unknown[] | null;
  worktrees?: unknown[] | null;
  selectedRepoId?: string | null;
  setup?: unknown;
}): Record<string, unknown> {
  const redacted: Record<string, unknown> = {
    worktrees: data.worktrees ?? undefined,
    selectedRepoId: data.selectedRepoId ?? undefined,
  };
  if (data.repos) {
    redacted.repos = data.repos.map((value) => {
      const repo = persistedRecord(value ?? {});
      return Object.fromEntries(
        LEGACY_REPO_FIELDS.filter((field) => repo[field] !== undefined).map((field) => [
          field,
          repo[field],
        ])
      );
    });
  }
  if (data.setup) {
    redacted.setup = { isComplete: persistedBoolean(persistedRecord(data.setup).isComplete) };
  }
  return redacted;
}

/**
 * The Linear keys, lifted out of `store.json` and kept in the keychain. They travel as
 * one blob rather than one keychain item each because the keychain ACL is per item, so
 * N items would mean N authorization prompts the first time a new build reads them.
 */
export interface SecretBundle {
  /** The global key from the setup wizard, used as a fallback when a workspace has none. */
  setup: string | null;
  /** Per-workspace keys, by workspace id. A workspace without a key is simply absent. */
  workspaces: Record<string, string>;
}

export const EMPTY_SECRETS: SecretBundle = { setup: null, workspaces: {} };

/** Gather the Linear keys held by a setup blob and a workspace list. */
export function collectSecrets(
  setup?: AppState["setup"] | null,
  workspaces?: Workspace[] | null
): SecretBundle {
  const collected: SecretBundle = { setup: setup?.linearApiKey ?? null, workspaces: {} };
  for (const workspace of workspaces ?? []) {
    if (workspace.linearApiKey) collected.workspaces[workspace.id] = workspace.linearApiKey;
  }
  return collected;
}

/** Whether anything is worth storing, so a fresh install can skip the keychain entirely. */
export function hasSecrets(secrets: SecretBundle): boolean {
  return secrets.setup != null || Object.keys(secrets.workspaces).length > 0;
}

/**
 * Combine two bundles, `preferred` winning per key. Used to reconcile the keychain with
 * whatever is still in the file, so a store caught mid-migration keeps every key it has.
 */
export function mergeSecrets(base: SecretBundle, preferred: SecretBundle): SecretBundle {
  return {
    setup: preferred.setup ?? base.setup,
    workspaces: { ...base.workspaces, ...preferred.workspaces },
  };
}

export function setupWithoutSecret(setup: AppState["setup"]): AppState["setup"] {
  return { ...setup, linearApiKey: null };
}

export function workspacesWithoutSecrets(workspaces: Workspace[]): Workspace[] {
  return workspaces.map((workspace) => ({ ...workspace, linearApiKey: null }));
}

export function setupWithSecret(
  setup: AppState["setup"],
  secrets: SecretBundle
): AppState["setup"] {
  return { ...setup, linearApiKey: secrets.setup };
}

export function workspacesWithSecrets(workspaces: Workspace[], secrets: SecretBundle): Workspace[] {
  return workspaces.map((workspace) => ({
    ...workspace,
    linearApiKey: secrets.workspaces[workspace.id] ?? null,
  }));
}

/** Normalize task objects loaded from the store, filling defaults. */
export function normalizeTasks(raw: unknown): Task[] {
  return persistedArray(raw).map((value) => {
    const t = persistedRecord(value);
    return {
      id: persistedId(t.id),
      workspaceId: persistedId(t.workspaceId),
      branchName: persistedString(t.branchName) ?? "",
      linearIssueId: persistedString(t.linearIssueId),
      linearIssueTitle: persistedString(t.linearIssueTitle),
      linearProjectId: persistedString(t.linearProjectId),
      linearProjectName: persistedString(t.linearProjectName),
      linearIssueIdentifier: persistedString(t.linearIssueIdentifier),
      workspaceFilePath: persistedString(t.workspaceFilePath) ?? null,
      noteFolder: persistedString(t.noteFolder),
      noteFileName: persistedString(t.noteFileName),
      pinOrder:
        typeof t.pinOrder === "number" && Number.isSafeInteger(t.pinOrder) && t.pinOrder >= 0
          ? t.pinOrder
          : undefined,
      createdAt: persistedString(t.createdAt) ?? new Date().toISOString(),
      members: persistedArray(t.members).map((value) => {
        const m = persistedRecord(value);
        return {
          repoId: persistedId(m.repoId),
          repoName: persistedString(m.repoName) ?? "",
          localPath: persistedString(m.localPath) ?? "",
          path: persistedString(m.path) ?? "",
          branchName: persistedString(m.branchName) ?? "",
        };
      }),
    };
  });
}

/**
 * One-time migration from the old single-repo schema (`repos` + `worktrees`) to the
 * multi-repo schema (`workspaces` + `tasks`). Each old repo becomes a single-member
 * workspace; each old worktree becomes a single-member task. IDs are preserved so the
 * old `selectedRepoId` maps directly onto the new `selectedWorkspaceId`.
 */
export function migrateLegacyToWorkspaces(
  rawRepos: unknown,
  rawWorktrees: unknown,
  globalLinearApiKey?: string | null
): { workspaces: Workspace[]; tasks: Task[] } {
  const repos = persistedArray(rawRepos).map(persistedRecord);
  const applyGlobal =
    !!globalLinearApiKey &&
    repos.length > 0 &&
    repos.every((r) => !persistedString(r.linearApiKey));

  const workspaces: Workspace[] = repos.map((r) => ({
    id: persistedId(r.id),
    name: persistedString(r.name) ?? "",
    linearApiKey: persistedString(r.linearApiKey) ?? (applyGlobal ? globalLinearApiKey : null),
    linearOrgUrlKey: null,
    repos: [
      {
        id: persistedId(r.id),
        name: persistedString(r.name) ?? "",
        localPath: persistedString(r.localPath) ?? "",
        worktreeBasePath: persistedString(r.worktreeBasePath) ?? "",
      },
    ],
  }));

  const repoById = new Map(repos.map((r) => [persistedId(r.id), r]));
  const tasks: Task[] = persistedArray(rawWorktrees).map((value) => {
    const w = persistedRecord(value);
    const r = repoById.get(persistedId(w.repoId));
    return {
      id: persistedId(w.id),
      workspaceId: persistedId(w.repoId),
      branchName: persistedString(w.branchName) ?? "",
      linearIssueId: persistedString(w.linearIssueId),
      linearIssueTitle: persistedString(w.linearIssueTitle),
      linearIssueIdentifier: persistedString(w.linearIssueIdentifier),
      workspaceFilePath: null,
      createdAt: persistedString(w.createdAt) ?? new Date().toISOString(),
      members: [
        {
          repoId: persistedId(w.repoId),
          repoName: persistedString(r?.name) ?? "",
          localPath: persistedString(r?.localPath) ?? "",
          path: persistedString(w.path) ?? "",
          branchName: persistedString(w.branchName) ?? "",
        },
      ],
    };
  });

  return { workspaces, tasks };
}

export function compareTaskPins(a: Task, b: Task): number {
  if (a.pinOrder === undefined) return b.pinOrder === undefined ? 0 : 1;
  if (b.pinOrder === undefined) return -1;
  return a.pinOrder - b.pinOrder;
}
