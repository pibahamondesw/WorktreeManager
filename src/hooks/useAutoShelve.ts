import { useEffect, useRef } from "react";
import { GitStatus, IssueLinearInfo, Task } from "../types";
import { useGithubPrStatuses } from "../services/github";
import { taskLinearStore } from "../services/taskLinear";
import { AutoShelveSettings, autoShelveReason, autoShelveSignals } from "../services/taskShelf";

interface Params {
  tasks: Task[];
  linearInfo: Record<string, IssueLinearInfo>;
  gitStatuses: Record<string, GitStatus>;
  settings: AutoShelveSettings;
  now: number;
  shelveTasks: (ids: string[]) => Promise<void>;
}

/** Moves the listed tasks whose work is over to Finished, once their signals are known. */
export function useAutoShelve({
  tasks,
  linearInfo,
  gitStatuses,
  settings,
  now,
  shelveTasks,
}: Params) {
  const prStatuses = useGithubPrStatuses();
  const requested = useRef(new Set<string>());

  useEffect(() => {
    const sources = {
      linearInfo,
      gitStatuses,
      repoSlugs: (task: Task) => taskLinearStore.repoSlugs(task.members),
    };
    const ids = tasks
      .filter(
        (task) =>
          !requested.current.has(task.id) &&
          autoShelveReason(task, autoShelveSignals(task, sources), now, settings) !== null
      )
      .map((task) => task.id);
    if (ids.length === 0) return;
    ids.forEach((id) => requested.current.add(id));
    void shelveTasks(ids)
      .catch(() => undefined)
      .finally(() => ids.forEach((id) => requested.current.delete(id)));
  }, [tasks, linearInfo, gitStatuses, settings, now, prStatuses, shelveTasks]);
}
