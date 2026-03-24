import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useLinear } from "../contexts/LinearContext";
import { Worktree, Repo, GitStatus, IssueLinearInfo } from "../types";

export function useWorktreeData(worktrees: Worktree[], repo: Repo | undefined, onReady?: () => void) {
  const [linearInfo, setLinearInfo] = useState<Record<string, IssueLinearInfo>>({});
  const [gitStatuses, setGitStatuses] = useState<Record<string, GitStatus>>({});
  const [refreshing, setRefreshing] = useState(false);
  const linear = useLinear();
  const onReadyRef = useRef(onReady);
  onReadyRef.current = onReady;

  const fetchLinearInfo = useCallback(async () => {
    const issueIds = worktrees
      .map((wt) => wt.linearIssueId)
      .filter((id): id is string => !!id);
    if (issueIds.length === 0 || !linear) {
      setLinearInfo({});
      return;
    }
    const info = await linear.fetchIssueLinearInfoBatch(issueIds);
    setLinearInfo(info);
  }, [worktrees, linear]);

  const fetchGitStatuses = useCallback(async () => {
    if (worktrees.length === 0 || !repo) {
      setGitStatuses({});
      return;
    }
    try {
      const statuses = await invoke<Record<string, GitStatus>>("git_worktree_status_batch", {
        worktreePaths: worktrees.map((wt) => wt.path),
        repoPath: repo.localPath,
      });
      setGitStatuses(statuses);
    } catch {
      /* git status is best-effort */
    }
  }, [worktrees, repo]);

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
