import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LinearService } from "../services/linear";
import { Task, Workspace, GitStatus, IssueLinearInfo } from "../types";

import { GITHUB_REFRESH_INTERVAL_MS, refreshGithubPrs } from "../services/github";

const AUTO_REFRESH_INTERVAL_MS = 15 * 60 * 1000;

export function useWorktreeData(
  tasks: Task[],
  workspace: Workspace | undefined,
  linear: LinearService | null,
  onReady?: (workspaceId?: string) => void
) {
  const [linearInfo, setLinearInfo] = useState<Record<string, IssueLinearInfo>>({});
  const [gitStatuses, setGitStatuses] = useState<Record<string, GitStatus>>({});
  const [refreshing, setRefreshing] = useState(false);
  const onReadyRef = useRef(onReady);
  onReadyRef.current = onReady;
  const lastRefreshRef = useRef(0);
  const prsRef = useRef<IssueLinearInfo["prs"]>([]);
  const lastGithubRefreshRef = useRef(0);
  const linearRequestRef = useRef(0);

  const invalidateLinearRequests = useCallback(() => {
    linearRequestRef.current++;
  }, []);

  const fetchLinearInfo = useCallback(async () => {
    const request = ++linearRequestRef.current;
    const issueIds = tasks.map((t) => t.linearIssueId).filter((id): id is string => !!id);
    if (issueIds.length === 0 || !linear) {
      prsRef.current = [];
      return;
    }
    const info = await linear.fetchIssueLinearInfoBatch([...new Set(issueIds)]).catch(() => {
      if (request === linearRequestRef.current) {
        lastGithubRefreshRef.current = Date.now();
        void refreshGithubPrs(prsRef.current, true);
      }
      return undefined;
    });
    if (!info || request !== linearRequestRef.current) return;
    // Merge instead of replace: entries are keyed by issue id, so results from
    // other workspaces stay cached and render instantly when switching back.
    setLinearInfo((prev) => ({ ...prev, ...info }));
    const prs = Object.values(info).flatMap((issue) => issue.prs);
    prsRef.current = prs;
    lastGithubRefreshRef.current = Date.now();
    void refreshGithubPrs(prs, true);
  }, [tasks, linear]);

  const fetchGitStatuses = useCallback(async () => {
    if (tasks.length === 0 || !workspace) return;
    // Group member worktree paths by their repo's main clone, then batch per repo.
    const pathsByRepo = new Map<string, string[]>();
    for (const task of tasks) {
      for (const m of task.members) {
        const list = pathsByRepo.get(m.localPath) ?? [];
        list.push(m.path);
        pathsByRepo.set(m.localPath, list);
      }
    }
    try {
      const results = await Promise.all(
        [...pathsByRepo.entries()].map(([repoPath, worktreePaths]) =>
          invoke<Record<string, GitStatus>>("git_worktree_status_batch", {
            worktreePaths,
            repoPath,
          })
        )
      );
      const merged: Record<string, GitStatus> = {};
      for (const r of results) Object.assign(merged, r);
      setGitStatuses((prev) => ({ ...prev, ...merged }));
    } catch {
      /* git status is best-effort */
    }
  }, [tasks, workspace]);

  const handleRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await Promise.allSettled([fetchLinearInfo(), fetchGitStatuses()]);
    } finally {
      lastRefreshRef.current = Date.now();
      setRefreshing(false);
    }
  }, [fetchLinearInfo, fetchGitStatuses]);

  const handleRefreshRef = useRef(handleRefresh);
  handleRefreshRef.current = handleRefresh;

  useEffect(() => {
    let stale = false;
    const workspaceId = workspace?.id;
    prsRef.current = [];
    setRefreshing(true);
    void Promise.allSettled([fetchLinearInfo(), fetchGitStatuses()]).finally(() => {
      if (stale) return;
      lastRefreshRef.current = Date.now();
      setRefreshing(false);
      onReadyRef.current?.(workspaceId);
    });
    return () => {
      stale = true;
      invalidateLinearRequests();
    };
  }, [fetchLinearInfo, fetchGitStatuses, workspace?.id, invalidateLinearRequests]);

  useEffect(() => {
    if (!workspace?.id) return;
    const refreshIfDue = () => {
      if (document.visibilityState !== "visible") return;
      if (Date.now() - lastGithubRefreshRef.current >= GITHUB_REFRESH_INTERVAL_MS) {
        lastGithubRefreshRef.current = Date.now();
        void refreshGithubPrs(prsRef.current);
      }
      if (Date.now() - lastRefreshRef.current < AUTO_REFRESH_INTERVAL_MS) return;
      void handleRefreshRef.current();
    };
    const timer = setInterval(refreshIfDue, 60 * 1000);
    document.addEventListener("visibilitychange", refreshIfDue);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshIfDue);
    };
  }, [workspace?.id]);

  return { linearInfo, gitStatuses, refreshing, handleRefresh };
}
