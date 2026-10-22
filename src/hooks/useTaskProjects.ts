import { useEffect } from "react";
import { Operations } from "../services/operations";
import { Task, Workspace } from "../types";
import { useTaskLinearData } from "./useTaskLinearData";

export function useTaskProjects(tasks: Task[], workspaces: Workspace[], operations: Operations) {
  const revision = useTaskLinearData(tasks, workspaces);
  const issues = JSON.stringify(tasks.map((task) => [task.workspaceId, task.linearIssueId]));
  useEffect(() => {
    for (const workspace of workspaces) {
      void operations.syncTaskProjects(workspace.id).catch(() => undefined);
    }
  }, [revision, issues, workspaces, operations]);
}
