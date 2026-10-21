import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Task } from "../types";
import {
  AgentActivities,
  AgentAlertSettings,
  AgentEvent,
  DEFAULT_AGENT_ALERTS,
  NotificationTarget,
  alertsFor,
  chatActivityEvent,
  endStaleSessions,
  markTaskSeen,
  nextActivity,
  notificationFor,
  pruneActivities,
  taskForCwd,
  pendingInputCount,
  settledAlerts,
} from "../services/agentActivity";
import { sessionRegistry } from "../embedded/sessionTransports";
import { onOpenCodeActivity } from "../services/opencode";
import { TerminalStatus } from "../services/terminal";
import { loadAgentAlerts, persist } from "../services/store";

const NOTIFICATION_OPEN_EVENT = "agent-notification-open";

interface UseAgentActivityArgs {
  tasks: Task[];
  sessions: Record<string, TerminalStatus>;
  openedTaskId: string | null;
  onOpenTarget: (task: Task, target: NotificationTarget) => void;
}

/**
 * Agent state per task: embedded chats report it directly, every other surface through the
 * global hooks (`wtm agent event`).
 */
export function useAgentActivity({
  tasks,
  sessions,
  openedTaskId,
  onOpenTarget,
}: UseAgentActivityArgs) {
  const [activities, setActivities] = useState<AgentActivities>({});
  const activitiesRef = useRef(activities);
  useEffect(() => {
    activitiesRef.current = activities;
  }, [activities]);
  const [alerts, setAlerts] = useState<AgentAlertSettings>(DEFAULT_AGENT_ALERTS);
  const latest = useRef({ tasks, openedTaskId, alerts, onOpenTarget });
  useEffect(() => {
    latest.current = { tasks, openedTaskId, alerts, onOpenTarget };
  }, [tasks, openedTaskId, alerts, onOpenTarget]);

  useEffect(() => {
    void loadAgentAlerts()
      .then(setAlerts)
      .catch(() => undefined);
  }, []);

  const updateAlerts = useCallback((next: AgentAlertSettings) => {
    setAlerts(next);
    persist([["agentAlerts", next]]).catch(() => undefined);
  }, []);

  const record = useCallback((task: Task, event: Omit<AgentEvent, "cwd" | "taskId">) => {
    const { openedTaskId, alerts } = latest.current;
    const viewing = task.id === openedTaskId && document.hasFocus();
    const current = activitiesRef.current[task.id];
    const next = nextActivity(current, event, viewing);
    if (!next) return;
    activitiesRef.current = { ...activitiesRef.current, [task.id]: next };
    setActivities((activities) => ({ ...activities, [task.id]: next }));
    const category = alertsFor(current, next);
    if (!category) return;
    void invoke("agent_attention", {
      sound: alerts.sounds[category],
      notification: alerts.notifications[category]
        ? {
            ...notificationFor(task, { ...event, state: category }),
            taskOpen: task.id === openedTaskId,
          }
        : null,
    }).catch(() => undefined);
  }, []);

  const handleAgentEvent = useCallback(
    (event: AgentEvent) => {
      const { tasks, alerts } = latest.current;
      const task = event.taskId
        ? tasks.find((t) => t.id === event.taskId)
        : event.cwd
          ? taskForCwd(tasks, event.cwd)
          : undefined;
      if (event.surface === "chat") return { taskId: task?.id ?? null };
      if (task) record(task, event);
      else if (event.state !== "working")
        void invoke("agent_attention", {
          sound: alerts.sounds[event.state],
          notification: null,
        }).catch(() => undefined);
      return { taskId: task?.id ?? null };
    },
    [record]
  );

  useEffect(
    () =>
      sessionRegistry.onSessionEvent("chat", (chat, previous) => {
        const event = chatActivityEvent(previous, chat, Date.now());
        if (event === "end")
          setActivities((current) => {
            if (current[chat.taskId]?.surface !== "chat") return current;
            const next = { ...current };
            delete next[chat.taskId];
            return next;
          });
        else if (event) {
          const task = latest.current.tasks.find((t) => t.id === chat.taskId);
          if (task) record(task, event);
        }
      }),
    [record]
  );

  useEffect(() => {
    const unlisten = onOpenCodeActivity(({ taskId, state, at }) => {
      const task = latest.current.tasks.find((t) => t.id === taskId);
      if (task) record(task, { state, agent: "opencode", surface: "opencode", at });
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [record]);

  useEffect(() => {
    setActivities((current) => pruneActivities(current, tasks));
  }, [tasks]);

  useEffect(() => {
    setActivities((current) => endStaleSessions(current, sessions));
  }, [sessions, activities]);

  useEffect(() => {
    if (!openedTaskId) return;
    const markSeen = () => setActivities((current) => markTaskSeen(current, openedTaskId));
    markSeen();
    window.addEventListener("focus", markSeen);
    return () => window.removeEventListener("focus", markSeen);
  }, [openedTaskId, activities]);

  const notifiedActivities = useRef(activities);
  useEffect(() => {
    const settled = settledAlerts(notifiedActivities.current, activities);
    notifiedActivities.current = activities;
    if (settled.length > 0)
      void invoke("agent_dismiss_notifications", { taskIds: settled }).catch(() => undefined);
  }, [activities]);

  const unread = pendingInputCount(activities);
  useEffect(() => {
    void invoke("agent_badge", { count: unread }).catch(() => undefined);
  }, [unread]);

  useEffect(() => {
    const unlisten = listen<NotificationTarget>(NOTIFICATION_OPEN_EVENT, ({ payload }) => {
      const task = latest.current.tasks.find((t) => t.id === payload.taskId);
      if (task) latest.current.onOpenTarget(task, payload);
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

  return { activities, handleAgentEvent, alerts, updateAlerts };
}
