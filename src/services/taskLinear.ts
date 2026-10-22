import { invoke } from "@tauri-apps/api/core";
import { IssueLinearInfo, PullRequestInfo, Task, TaskMember, Workspace } from "../types";
import { githubSlugFromRemote } from "../utils";
import { refreshGithubPrs } from "./github";
import { LinearService } from "./linear";

export const LINEAR_REFRESH_INTERVAL_MS = 15 * 60 * 1000;

type LinearInfo = Record<string, IssueLinearInfo>;

export interface TaskLinearPorts {
  linear: (apiKey: string) => {
    fetchIssueLinearInfoBatch: (issueIds: string[]) => Promise<LinearInfo>;
  };
  resolveSlug: (localPath: string) => Promise<string | null>;
  refreshPrs: (prs: PullRequestInfo[], force: boolean) => Promise<void>;
}

interface Account {
  info: LinearInfo;
  fetchedAt: Map<string, number>;
  pending: Map<string, Promise<LinearInfo>>;
  batch?: { ids: Set<string>; promise: Promise<LinearInfo> };
}

export function groupTaskPrs(
  task: Task,
  prs: PullRequestInfo[],
  repoSlugs: Record<string, string> | null
) {
  const byMember: Record<string, PullRequestInfo[]> = {};
  const memberSlugs = new Set<string>();
  for (const member of task.members) {
    const slug = repoSlugs?.[member.repoId]?.toLowerCase();
    byMember[member.repoId] = slug ? prs.filter((pr) => pr.repoSlug.toLowerCase() === slug) : [];
    if (slug) memberSlugs.add(slug);
  }
  const unmatched = new Map<string, PullRequestInfo[]>();
  if (repoSlugs !== null) {
    for (const pr of prs) {
      const slug = pr.repoSlug.toLowerCase();
      if (!memberSlugs.has(slug)) unmatched.set(slug, [...(unmatched.get(slug) ?? []), pr]);
    }
  }
  return { byMember, unmatched };
}

export class TaskLinearStore {
  private accounts = new Map<string, Account>();
  private slugs = new Map<string, string | null>();
  private slugRequests = new Map<string, Promise<string | null>>();
  private listeners = new Set<() => void>();
  private watches = new Set<{
    tasks: Task[];
    workspaces: Workspace[];
    onRefresh?: () => void;
    refreshedAt: number;
  }>();
  private timer?: ReturnType<typeof setInterval>;
  private revision = 0;

  private ports: TaskLinearPorts;

  constructor(ports: TaskLinearPorts) {
    this.ports = ports;
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getSnapshot = () => this.revision;

  private publish() {
    this.revision++;
    this.listeners.forEach((listener) => listener());
  }

  info(workspace: Workspace | undefined): LinearInfo {
    return (workspace?.linearApiKey && this.accounts.get(workspace.linearApiKey)?.info) || {};
  }

  taskInfo(task: Task, workspace: Workspace | undefined) {
    return task.linearIssueId ? this.info(workspace)[task.linearIssueId] : undefined;
  }

  repoSlugs(repos: Pick<TaskMember, "repoId" | "localPath">[]) {
    if (repos.some((repo) => !this.slugs.has(repo.localPath))) return null;
    return Object.fromEntries(
      repos.flatMap((repo) => {
        const slug = this.slugs.get(repo.localPath);
        return slug ? [[repo.repoId, slug]] : [];
      })
    );
  }

  taskPrs(task: Task, workspace: Workspace | undefined) {
    return groupTaskPrs(
      task,
      this.taskInfo(task, workspace)?.prs ?? [],
      this.repoSlugs(task.members)
    );
  }

  memberPrs(task: Task, member: TaskMember, workspace: Workspace) {
    return this.taskPrs({ ...task, members: [member] }, workspace).byMember[member.repoId] ?? [];
  }

  resolveSlug(localPath: string): Promise<string | null> {
    if (this.slugs.has(localPath)) return Promise.resolve(this.slugs.get(localPath)!);
    const active = this.slugRequests.get(localPath);
    if (active) return active;
    const request = this.ports
      .resolveSlug(localPath)
      .then(
        (slug) => {
          if (this.slugRequests.get(localPath) !== request) return null;
          this.slugs.set(localPath, slug?.toLowerCase() ?? null);
          this.publish();
          return slug?.toLowerCase() ?? null;
        },
        () => {
          if (this.slugRequests.get(localPath) === request) {
            this.slugs.set(localPath, null);
            this.publish();
          }
          return null;
        }
      )
      .finally(() => {
        if (this.slugRequests.get(localPath) === request) this.slugRequests.delete(localPath);
      });
    this.slugRequests.set(localPath, request);
    return request;
  }

  private account(apiKey: string) {
    let account = this.accounts.get(apiKey);
    if (!account) {
      account = { info: {}, fetchedAt: new Map(), pending: new Map() };
      this.accounts.set(apiKey, account);
    }
    return account;
  }

  async refreshIssues(
    workspace: Workspace,
    issueIds: string[],
    force = false
  ): Promise<LinearInfo> {
    const apiKey = workspace.linearApiKey;
    if (!apiKey || !issueIds.length) return {};
    const account = this.account(apiKey);
    const ids = [...new Set(issueIds)];
    const requests = new Set<Promise<LinearInfo>>();
    const cached: LinearInfo = {};
    for (const id of ids) {
      const active = account.pending.get(id);
      if (active) {
        requests.add(active);
        continue;
      }
      const fetchedAt = account.fetchedAt.get(id);
      if (
        !force &&
        fetchedAt !== undefined &&
        Date.now() - fetchedAt < LINEAR_REFRESH_INTERVAL_MS
      ) {
        if (account.info[id]) cached[id] = account.info[id];
        continue;
      }
      if (!account.batch) {
        const batchIds = new Set<string>();
        const promise = Promise.resolve().then(async () => {
          account.batch = undefined;
          try {
            const info = await this.ports.linear(apiKey).fetchIssueLinearInfoBatch([...batchIds]);
            if (this.accounts.get(apiKey) !== account) return {};
            account.info = { ...account.info, ...info };
            for (const issueId of batchIds) {
              if (info[issueId]) account.fetchedAt.set(issueId, Date.now());
            }
            this.publish();
            return info;
          } finally {
            for (const issueId of batchIds) account.pending.delete(issueId);
          }
        });
        account.batch = { ids: batchIds, promise };
      }
      account.batch.ids.add(id);
      account.pending.set(id, account.batch.promise);
      requests.add(account.batch.promise);
    }
    const results = await Promise.all(requests);
    const combined = Object.assign(cached, ...results) as LinearInfo;
    return Object.fromEntries(ids.flatMap((id) => (combined[id] ? [[id, combined[id]]] : [])));
  }

  async refreshWorkspace(workspace: Workspace, tasks: Task[], force = false) {
    const linked = tasks.filter((task) => task.workspaceId === workspace.id && task.linearIssueId);
    try {
      return await this.refreshIssues(
        workspace,
        linked.map((task) => task.linearIssueId!),
        force
      );
    } finally {
      const prs = linked.flatMap((task) => this.taskInfo(task, workspace)?.prs ?? []);
      void this.ports.refreshPrs(prs, force).catch(() => undefined);
    }
  }

  watch(tasks: Task[], workspaces: Workspace[], onRefresh?: () => void) {
    if (!workspaces.length) return () => {};
    const watch = { tasks, workspaces, onRefresh, refreshedAt: Date.now() };
    this.watches.add(watch);
    if (!this.timer) {
      this.timer = setInterval(this.refreshIfVisible, 60 * 1000);
      document.addEventListener("visibilitychange", this.refreshIfVisible);
    }
    return () => {
      this.watches.delete(watch);
      if (!this.watches.size) {
        clearInterval(this.timer);
        this.timer = undefined;
        document.removeEventListener("visibilitychange", this.refreshIfVisible);
      }
    };
  }

  private refreshIfVisible = () => {
    if (document.visibilityState !== "visible") return;
    for (const watch of this.watches) {
      for (const workspace of watch.workspaces) {
        void this.refreshWorkspace(workspace, watch.tasks).catch(() => undefined);
      }
      if (Date.now() - watch.refreshedAt >= LINEAR_REFRESH_INTERVAL_MS) {
        watch.refreshedAt = Date.now();
        watch.onRefresh?.();
      }
    }
  };

  reset() {
    this.accounts.clear();
    this.slugs.clear();
    this.slugRequests.clear();
    this.publish();
  }
}

export const taskLinearStore = new TaskLinearStore({
  linear: (apiKey) => new LinearService(apiKey),
  resolveSlug: async (localPath) =>
    githubSlugFromRemote(await invoke<string>("git_remote_url", { repoPath: localPath })),
  refreshPrs: refreshGithubPrs,
});
