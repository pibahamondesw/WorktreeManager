import { useEffect, useState } from "react";
import { LinearService } from "../services/linear";
import { Task, Workspace } from "../types";

export function useSearchPrNumbers(open: boolean, tasks: Task[], workspaces: Workspace[]) {
  const [prNumbersByTask, setPrNumbersByTask] = useState<Record<string, number[]>>({});

  useEffect(() => {
    if (!open) return;
    let cancelled = false;

    void Promise.all(
      workspaces.map(async (workspace): Promise<Record<string, number[]>> => {
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
            linkedTasks.map((task) => [
              task.id,
              info[task.linearIssueId!]?.prs.map((pr) => pr.number) ?? [],
            ])
          );
        } catch {
          return {};
        }
      })
    ).then((results) => {
      if (!cancelled)
        setPrNumbersByTask(Object.fromEntries(results.flatMap((result) => Object.entries(result))));
    });

    return () => {
      cancelled = true;
    };
  }, [open, tasks, workspaces]);

  return prNumbersByTask;
}
