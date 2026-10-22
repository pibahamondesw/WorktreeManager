import { taskLinearStore } from "../services/taskLinear";
import { PullRequestInfo, Task, Workspace } from "../types";
import { useTaskLinearData } from "./useTaskLinearData";

const NO_TASKS: Task[] = [];
const NO_WORKSPACES: Workspace[] = [];

export function useSearchPrNumbers(open: boolean, tasks: Task[], workspaces: Workspace[]) {
  useTaskLinearData(open ? tasks : NO_TASKS, open ? workspaces : NO_WORKSPACES);
  const prsByTask: Record<string, PullRequestInfo[]> = {};
  for (const task of tasks) {
    const workspace = workspaces.find((workspace) => workspace.id === task.workspaceId);
    prsByTask[task.id] = taskLinearStore.taskInfo(task, workspace)?.prs ?? [];
  }
  return {
    prsByTask,
    prNumbersByTask: Object.fromEntries(
      Object.entries(prsByTask).map(([taskId, prs]) => [taskId, prs.map((pr) => pr.number)])
    ),
  };
}
