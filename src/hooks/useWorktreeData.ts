import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LinearService } from "../services/linear";
import { Task, Workspace, GitStatus, IssueLinearInfo } from "../types";

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

  const fetchLinearInfo = useCallback(async () => {
    const issueIds = tasks
      .map((t) => t.linearIssueId)
      .filter((id): id is string => !!id);
    if (issueIds.length === 0 || !linear) return;
    const info = await linear.fetchIssueLinearInfoBatch(issueIds);
    // Merge instead of replace: entries are keyed by issue id, so results from
    // other workspaces stay cached and render instantly when switching back.
    setLinearInfo((prev) => ({ ...prev, ...info }));
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
      await Promise.all([fetchLinearInfo(), fetchGitStatuses()]);
    } finally {
      setRefreshing(false);
    }
  }, [fetchLinearInfo, fetchGitStatuses]);

  useEffect(() => {
    let stale = false;
    const workspaceId = workspace?.id;
    setRefreshing(true);
    Promise.all([fetchLinearInfo(), fetchGitStatuses()]).finally(() => {
      if (stale) return;
      setRefreshing(false);
      onReadyRef.current?.(workspaceId);
    });
    return () => { stale = true; };
  }, [fetchLinearInfo, fetchGitStatuses, workspace?.id]);

  return { linearInfo, gitStatuses, refreshing, handleRefresh };
}
