import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Task, Workspace, GitStatus } from "../types";
import { taskLinearStore } from "../services/taskLinear";
import { onWorktreeGitStatusChanged, watchWorktreeGitStatus } from "../services/gitStatusWatch";
import { useTaskLinearData } from "./useTaskLinearData";

function sameGitStatus(a: GitStatus, b: GitStatus) {
  return (
    a.ahead === b.ahead &&
    a.behind === b.behind &&
    a.dirty === b.dirty &&
    a.last_commit_epoch === b.last_commit_epoch
  );
}

export function mergeGitStatuses(
  prev: Record<string, GitStatus>,
  fetched: Record<string, GitStatus>
): Record<string, GitStatus> {
  const changed = Object.entries(fetched).filter(
    ([path, status]) => !prev[path] || !sameGitStatus(prev[path], status)
  );
  return changed.length === 0 ? prev : { ...prev, ...Object.fromEntries(changed) };
}

export function useWorktreeData(
  tasks: Task[],
  workspace: Workspace | undefined,
  onReady?: (workspaceId?: string) => void
) {
  const [gitStatuses, setGitStatuses] = useState<Record<string, GitStatus>>({});
  const [refreshing, setRefreshing] = useState(false);
  const onReadyRef = useRef(onReady);
  onReadyRef.current = onReady;
  const workspaces = useMemo(() => (workspace ? [workspace] : []), [workspace]);
  const requestCount = useRef(0);
  const latestRequestByPath = useRef(new Map<string, number>());

  const worktreePathsKey = useMemo(
    () =>
      JSON.stringify(
        workspace ? tasks.flatMap((task) => task.members.map((member) => member.path)).sort() : []
      ),
    [tasks, workspace]
  );

  const fetchGitStatuses = useCallback(
    async (onlyPaths?: ReadonlySet<string>) => {
      if (tasks.length === 0 || !workspace) return;
      // Group member worktree paths by their repo's main clone, then batch per repo.
      const pathsByRepo = new Map<string, string[]>();
      for (const task of tasks) {
        for (const m of task.members) {
          if (onlyPaths && !onlyPaths.has(m.path)) continue;
          const list = pathsByRepo.get(m.localPath) ?? [];
          list.push(m.path);
          pathsByRepo.set(m.localPath, list);
        }
      }
      const request = ++requestCount.current;
      for (const paths of pathsByRepo.values()) {
        for (const path of paths) latestRequestByPath.current.set(path, request);
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
        for (const path of Object.keys(merged)) {
          if (latestRequestByPath.current.get(path) !== request) delete merged[path];
        }
        setGitStatuses((prev) => mergeGitStatuses(prev, merged));
      } catch {
        /* git status is best-effort */
      }
    },
    [tasks, workspace]
  );

  const refreshGitStatuses = useCallback(() => {
    void fetchGitStatuses();
  }, [fetchGitStatuses]);
  useTaskLinearData(tasks, workspaces, refreshGitStatuses);

  const handleRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await Promise.allSettled([
        workspace ? taskLinearStore.refreshWorkspace(workspace, tasks, true) : Promise.resolve(),
        fetchGitStatuses(),
      ]);
    } finally {
      setRefreshing(false);
    }
  }, [tasks, workspace, fetchGitStatuses]);

  useEffect(() => {
    watchWorktreeGitStatus(JSON.parse(worktreePathsKey) as string[]);
  }, [worktreePathsKey]);

  useEffect(() => () => watchWorktreeGitStatus([]), []);

  useEffect(
    () => onWorktreeGitStatusChanged((paths) => void fetchGitStatuses(paths)),
    [fetchGitStatuses]
  );

  useEffect(() => {
    let stale = false;
    const workspaceId = workspace?.id;
    setRefreshing(true);
    void Promise.allSettled([
      workspace ? taskLinearStore.refreshWorkspace(workspace, tasks) : Promise.resolve(),
      fetchGitStatuses(),
    ]).finally(() => {
      if (stale) return;
      setRefreshing(false);
      onReadyRef.current?.(workspaceId);
    });
    return () => {
      stale = true;
    };
  }, [tasks, workspace, fetchGitStatuses]);

  return { linearInfo: taskLinearStore.info(workspace), gitStatuses, refreshing, handleRefresh };
}
