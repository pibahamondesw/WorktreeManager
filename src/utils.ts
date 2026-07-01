import { Task, Workspace } from "./types";

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
  globalLinearApiKey?: string | null,
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
