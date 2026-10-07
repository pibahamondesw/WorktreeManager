import { refreshGithubPrs } from "../services/github";
import { useEffect, useState } from "react";
import { LinearService } from "../services/linear";
import { PullRequestInfo, Task, Workspace } from "../types";

export function useSearchPrNumbers(open: boolean, tasks: Task[], workspaces: Workspace[]) {
  const [prsByTask, setPrsByTask] = useState<Record<string, PullRequestInfo[]>>({});

  useEffect(() => {
    if (!open) return;
    let cancelled = false;

    void Promise.all(
      workspaces.map(async (workspace): Promise<Record<string, PullRequestInfo[]>> => {
        if (!workspace.linearApiKey) return {};
        const linkedTasks = tasks.filter(
          (task) => task.workspaceId === workspace.id && task.linearIssueId
        );
        const issueIds = [...new Set(linkedTasks.map((task) => task.linearIssueId!))];
        if (issueIds.length === 0) return {};
        try {
          const info = await new LinearService(workspace.linearApiKey).fetchIssueLinearInfoBatch(
            issueIds
          );
          return Object.fromEntries(
            linkedTasks.map((task) => [task.id, info[task.linearIssueId!]?.prs ?? []])
          );
        } catch {
          return {};
        }
      })
    ).then((results) => {
      if (cancelled) return;
      const prs = Object.fromEntries(results.flatMap((result) => Object.entries(result)));
      setPrsByTask(prs);
      void refreshGithubPrs(Object.values(prs).flat());
    });

    return () => {
      cancelled = true;
    };
  }, [open, tasks, workspaces]);

  return {
    prsByTask,
    prNumbersByTask: Object.fromEntries(
      Object.entries(prsByTask).map(([taskId, prs]) => [taskId, prs.map((pr) => pr.number)])
    ),
  };
}
