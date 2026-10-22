import { useEffect, useSyncExternalStore } from "react";
import { taskLinearStore } from "../services/taskLinear";
import { Task, Workspace } from "../types";

export function useTaskLinearData(tasks: Task[], workspaces: Workspace[], onRefresh?: () => void) {
  const revision = useSyncExternalStore(taskLinearStore.subscribe, taskLinearStore.getSnapshot);
  useEffect(() => {
    for (const workspace of workspaces) {
      void taskLinearStore.refreshWorkspace(workspace, tasks).catch(() => undefined);
    }
    const paths = new Set(tasks.flatMap((task) => task.members.map((member) => member.localPath)));
    for (const path of paths) void taskLinearStore.resolveSlug(path);
    return taskLinearStore.watch(tasks, workspaces, onRefresh);
  }, [tasks, workspaces, onRefresh]);
  return revision;
}
