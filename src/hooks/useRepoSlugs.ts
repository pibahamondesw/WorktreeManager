import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Workspace } from "../types";

/** "owner/repo" from a GitHub remote URL, lowercased; null for non-GitHub remotes. */
export function githubSlugFromRemote(remoteUrl: string): string | null {
  const match = remoteUrl.match(/github\.com[/:]([^/]+\/[^/]+?)(?:\.git)?$/);
  return match ? match[1].toLowerCase() : null;
}

const slugByRepoPath = new Map<string, Promise<string | null>>();

function resolveSlug(localPath: string): Promise<string | null> {
  let pending = slugByRepoPath.get(localPath);
  if (!pending) {
    pending = invoke<string>("git_remote_url", { repoPath: localPath })
      .then(githubSlugFromRemote)
      .catch(() => null);
    slugByRepoPath.set(localPath, pending);
  }
  return pending;
}

export function resetRepoSlugCache(): void {
  slugByRepoPath.clear();
}

/**
 * GitHub slug of each workspace repo keyed by repoId, resolved once per repo path for the
 * whole session so switching workspaces never re-runs `git remote`. `null` while loading.
 */
export function useRepoSlugs(workspace: Workspace | undefined): Record<string, string> | null {
  const [slugs, setSlugs] = useState<Record<string, string> | null>(null);
  const repos = workspace?.repos;

  useEffect(() => {
    if (!repos) {
      setSlugs(null);
      return;
    }
    let cancelled = false;
    setSlugs(null);
    Promise.all(repos.map(async (r) => [r.id, await resolveSlug(r.localPath)] as const)).then(
      (entries) => {
        if (cancelled) return;
        const next: Record<string, string> = {};
        for (const [repoId, slug] of entries) {
          if (slug) next[repoId] = slug;
        }
        setSlugs(next);
      }
    );
    return () => {
      cancelled = true;
    };
  }, [repos]);

  return slugs;
}
