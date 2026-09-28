import { useEffect } from "react";
import { Operations } from "../services/operations";
import { Task, Workspace } from "../types";

export function useTaskProjects(tasks: Task[], workspaces: Workspace[], operations: Operations) {
  const issues = JSON.stringify(tasks.map((task) => [task.workspaceId, task.linearIssueId]));
  useEffect(() => {
    const refresh = () => {
      for (const workspace of workspaces) {
        if (!workspace.linearApiKey) continue;
        void operations.refreshTaskProjects(workspace.id).catch(() => undefined);
      }
    };
    refresh();
    const timer = setInterval(refresh, 15 * 60 * 1000);
    return () => clearInterval(timer);
  }, [issues, workspaces, operations]);
}
