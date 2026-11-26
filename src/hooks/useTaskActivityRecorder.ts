import { useEffect, useRef } from "react";
import { Task } from "../types";
import { AgentActivities } from "../services/agentActivity";

const RECORD_INTERVAL_MS = 60 * 60 * 1000;

interface Params {
  tasks: Task[];
  lastVisitAt: ReadonlyMap<string, string>;
  agentActivities: AgentActivities;
  record: (activity: Record<string, string>) => Promise<void>;
}

/** Saves the latest visit or agent event of each task, at most hourly, so inactivity survives restarts. */
export function useTaskActivityRecorder({ tasks, lastVisitAt, agentActivities, record }: Params) {
  const pending = useRef(false);

  useEffect(() => {
    if (pending.current) return;
    const activity: Record<string, string> = {};
    for (const task of tasks) {
      const seenAt = Math.max(
        Date.parse(lastVisitAt.get(task.id) ?? "") || 0,
        agentActivities[task.id]?.at ?? 0
      );
      const savedAt = Date.parse(task.lastActiveAt ?? "") || 0;
      if (seenAt - savedAt >= RECORD_INTERVAL_MS)
        activity[task.id] = new Date(seenAt).toISOString();
    }
    if (Object.keys(activity).length === 0) return;
    pending.current = true;
    void record(activity)
      .catch(() => undefined)
      .finally(() => {
        pending.current = false;
      });
  }, [tasks, lastVisitAt, agentActivities, record]);
}
