import { invoke } from "@tauri-apps/api/core";
import { useSyncExternalStore } from "react";
import { GithubPrStatus, PullRequestInfo } from "../types";

export const GITHUB_REFRESH_INTERVAL_MS = 5 * 60 * 1000;
interface Entry {
  status?: GithubPrStatus;
  fetchedAt: number;
  pendingReady?: boolean;
}
let entries: Record<string, Entry> = {};
const listeners = new Set<() => void>();
const generations = new Map<string, number>();
const requests = new Map<string, { numbers: Set<number>; promise: Promise<void> }>();

export function prKey(pr: Pick<PullRequestInfo, "repoSlug" | "number">): string {
  return `${pr.repoSlug.toLowerCase()}#${pr.number}`;
}

function publish(next: Record<string, Entry>) {
  entries = next;
  listeners.forEach((listener) => listener());
}

export function useGithubPrStatuses() {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    () => entries
  );
}

export function githubPrStatus(pr: PullRequestInfo): GithubPrStatus | undefined {
  return entries[prKey(pr)]?.status;
}

export function setPrReadyPending(pr: PullRequestInfo, pendingReady: boolean) {
  const key = prKey(pr);
  publish({
    ...entries,
    [key]: { ...entries[key], fetchedAt: entries[key]?.fetchedAt ?? 0, pendingReady },
  });
}

export function invalidateGithubRepo(repoSlug: string, readyPr?: PullRequestInfo) {
  const slug = repoSlug.toLowerCase();
  generations.set(slug, (generations.get(slug) ?? 0) + 1);
  requests.delete(slug);
  const next = { ...entries };
  for (const key of Object.keys(next)) {
    if (key.startsWith(`${slug}#`)) next[key] = { ...next[key], fetchedAt: 0 };
  }
  if (readyPr && next[prKey(readyPr)]?.status) {
    const key = prKey(readyPr);
    next[key] = { ...next[key], status: { ...next[key].status!, isDraft: false } };
  }
  publish(next);
}

async function fetchRepo(slug: string, numbers: number[], force: boolean): Promise<void> {
  const active = requests.get(slug);
  if (active) {
    await active.promise;
    const remaining = numbers.filter((number) => !active.numbers.has(number));
    if (remaining.length) await fetchRepo(slug, remaining, force);
    return;
  }
  const due = numbers.filter(
    (number) =>
      force ||
      !entries[`${slug}#${number}`] ||
      Date.now() - entries[`${slug}#${number}`].fetchedAt >= GITHUB_REFRESH_INTERVAL_MS
  );
  if (!due.length) return;
  const generation = generations.get(slug) ?? 0;
  const promise = (async () => {
    let statuses: Record<string, GithubPrStatus> = {};
    try {
      statuses =
        (await invoke<Record<string, GithubPrStatus>>("github_pr_status_batch", {
          repoSlug: slug,
          prNumbers: due,
        })) ?? {};
    } catch {
      statuses = {};
    }
    if (generation !== (generations.get(slug) ?? 0)) return;
    const next = { ...entries };
    for (const number of due) {
      const key = `${slug}#${number}`;
      next[key] = { ...next[key], fetchedAt: Date.now(), status: statuses[number] };
    }
    publish(next);
  })();
  requests.set(slug, { numbers: new Set(due), promise });
  try {
    await promise;
  } finally {
    if (requests.get(slug)?.promise === promise) requests.delete(slug);
  }
}

export async function refreshGithubPrs(prs: PullRequestInfo[], force = false): Promise<void> {
  const repos = new Map<string, Set<number>>();
  for (const pr of prs) {
    const slug = pr.repoSlug.toLowerCase();
    const numbers = repos.get(slug) ?? new Set<number>();
    numbers.add(pr.number);
    repos.set(slug, numbers);
  }
  await Promise.all([...repos].map(([slug, numbers]) => fetchRepo(slug, [...numbers], force)));
}

export async function refreshGithubRepo(repoSlug: string): Promise<void> {
  const slug = repoSlug.toLowerCase();
  const numbers = Object.keys(entries)
    .filter((key) => key.startsWith(`${slug}#`))
    .map((key) => Number(key.slice(slug.length + 1)));
  await fetchRepo(slug, numbers, true);
}

export function resetGithubCache() {
  for (const slug of requests.keys()) generations.set(slug, (generations.get(slug) ?? 0) + 1);
  requests.clear();
  publish({});
}
