import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LinearService } from "../services/linear";
import { Task, Workspace, GitStatus, IssueLinearInfo } from "../types";

export function useWorktreeData(
  tasks: Task[],
  workspace: Workspace | undefined,
  linear: LinearService | null,
  onReady?: () => void
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
    if (issueIds.length === 0 || !linear) {
      setLinearInfo({});
      return;
    }
    const info = await linear.fetchIssueLinearInfoBatch(issueIds);
    setLinearInfo(info);
  }, [tasks, linear]);

  const fetchGitStatuses = useCallback(async () => {
    if (tasks.length === 0 || !workspace) {
      setGitStatuses({});
      return;
    }
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
      setGitStatuses(merged);
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
    Promise.all([fetchLinearInfo(), fetchGitStatuses()])
      .finally(() => { if (!stale) onReadyRef.current?.(); });
    return () => { stale = true; };
  }, [fetchLinearInfo, fetchGitStatuses]);

  return { linearInfo, gitStatuses, refreshing, handleRefresh };
}
