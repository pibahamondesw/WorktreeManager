import { Repo } from "./types";

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
 * Normalize raw repo objects from the store, filling in defaults for
 * fields that may be missing in data from older versions.
 */
export function migrateRepos(rawRepos: any[], globalLinearApiKey?: string | null): Repo[] {
  const repos = rawRepos.map((r: any) => ({
    id: r.id,
    name: r.name,
    localPath: r.localPath ?? "",
    worktreeBasePath: r.worktreeBasePath ?? "",
    linearApiKey: r.linearApiKey ?? null,
  }));

  if (globalLinearApiKey && repos.length > 0 && repos.every((r) => !r.linearApiKey)) {
    for (const r of repos) r.linearApiKey = globalLinearApiKey;
  }

  return repos;
}
