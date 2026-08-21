import { AppState, Task, Workspace } from "./types";

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

/**
 * Normalize workspace objects loaded from the store, filling defaults for fields
 * that may be missing in data written by older builds.
 */
export function normalizeWorkspaces(raw: any[] | undefined | null): Workspace[] {
  return (raw ?? []).map((w: any) => ({
    id: w.id,
    name: w.name ?? "",
    linearApiKey: w.linearApiKey ?? null,
    repos: (w.repos ?? []).map((r: any) => ({
      id: r.id,
      name: r.name ?? "",
      localPath: r.localPath ?? "",
      worktreeBasePath: r.worktreeBasePath ?? "",
    })),
  }));
}

/**
 * Reduce the persisted `setup` blob to the fields the app still uses. The store
 * hands back whatever it finds on disk, so stale keys survive unless dropped
 * explicitly — notably `githubToken`, written by builds from before PR info moved
 * to Linear attachments and never read since.
 */
export function normalizeSetup(raw: any): AppState["setup"] {
  return {
    linearApiKey: raw?.linearApiKey ?? null,
    isComplete: raw?.isComplete ?? false,
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
  repos?: any[] | null;
  worktrees?: any[] | null;
  selectedRepoId?: string | null;
  setup?: any;
}): Record<string, unknown> {
  const redacted: Record<string, unknown> = {
    worktrees: data.worktrees ?? undefined,
    selectedRepoId: data.selectedRepoId ?? undefined,
  };
  if (data.repos) {
    redacted.repos = data.repos.map((repo: any) =>
      Object.fromEntries(
        LEGACY_REPO_FIELDS.filter((field) => repo?.[field] !== undefined).map((field) => [
          field,
          repo[field],
        ])
      )
    );
  }
  if (data.setup) {
    redacted.setup = { isComplete: data.setup.isComplete ?? false };
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
export function normalizeTasks(raw: any[] | undefined | null): Task[] {
  return (raw ?? []).map((t: any) => ({
    id: t.id,
    workspaceId: t.workspaceId,
    branchName: t.branchName ?? "",
    linearIssueId: t.linearIssueId,
    linearIssueTitle: t.linearIssueTitle,
    linearIssueIdentifier: t.linearIssueIdentifier,
    workspaceFilePath: t.workspaceFilePath ?? null,
    createdAt: t.createdAt ?? new Date().toISOString(),
    members: (t.members ?? []).map((m: any) => ({
      repoId: m.repoId,
      repoName: m.repoName ?? "",
      localPath: m.localPath ?? "",
      path: m.path ?? "",
      branchName: m.branchName ?? "",
    })),
  }));
}

/**
 * One-time migration from the old single-repo schema (`repos` + `worktrees`) to the
 * multi-repo schema (`workspaces` + `tasks`). Each old repo becomes a single-member
 * workspace; each old worktree becomes a single-member task. IDs are preserved so the
 * old `selectedRepoId` maps directly onto the new `selectedWorkspaceId`.
 */
export function migrateLegacyToWorkspaces(
  rawRepos: any[] | undefined | null,
  rawWorktrees: any[] | undefined | null,
  globalLinearApiKey?: string | null
): { workspaces: Workspace[]; tasks: Task[] } {
  const repos = rawRepos ?? [];
  const applyGlobal =
    !!globalLinearApiKey && repos.length > 0 && repos.every((r: any) => !r.linearApiKey);

  const workspaces: Workspace[] = repos.map((r: any) => ({
    id: r.id,
    name: r.name ?? "",
    linearApiKey: r.linearApiKey ?? (applyGlobal ? globalLinearApiKey : null),
    repos: [
      {
        id: r.id,
        name: r.name ?? "",
        localPath: r.localPath ?? "",
        worktreeBasePath: r.worktreeBasePath ?? "",
      },
    ],
  }));

  const repoById = new Map<string, any>(repos.map((r: any) => [r.id, r]));
  const tasks: Task[] = (rawWorktrees ?? []).map((w: any) => {
    const r = repoById.get(w.repoId);
    return {
      id: w.id,
      workspaceId: w.repoId,
      branchName: w.branchName ?? "",
      linearIssueId: w.linearIssueId,
      linearIssueTitle: w.linearIssueTitle,
      linearIssueIdentifier: w.linearIssueIdentifier,
      workspaceFilePath: null,
      createdAt: w.createdAt ?? new Date().toISOString(),
      members: [
        {
          repoId: w.repoId,
          repoName: r?.name ?? "",
          localPath: r?.localPath ?? "",
          path: w.path ?? "",
          branchName: w.branchName ?? "",
        },
      ],
    };
  });

  return { workspaces, tasks };
}
