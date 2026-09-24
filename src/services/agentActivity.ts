import { invoke } from "@tauri-apps/api/core";
import { AgentId, Task, TaskSurface } from "../types";
import { TerminalStatus } from "./terminal";
import { ChatInfo, isLive } from "./chat";

export type AgentActivityState = "working" | "waiting" | "done";

/** Where the agent runs: an embedded surface, or outside the app (Cursor, a terminal). */
export type AgentSurfaceKind = "chat" | "terminal" | "editor" | "external";

export interface AgentActivity {
  state: AgentActivityState;
  agent: AgentId;
  surface: AgentSurfaceKind;
  /** A finished or waiting agent stays unread until its task is opened. */
  unread: boolean;
  /** When the reported event happened, in ms; older events never overwrite newer ones. */
  at: number;
  /** A non-blocking question is still unanswered, so the agent keeps waiting on you. */
  openQuestion?: boolean;
}

export type AgentActivities = Record<string, AgentActivity>;

export interface AgentEvent {
  state: AgentActivityState;
  agent: AgentId;
  cwd?: string;
  at: number;
  /** Set by embedded sessions, which know their task; external agents are matched by cwd. */
  taskId?: string;
  surface: AgentSurfaceKind;
  /** The user submitted a prompt, which answers any open question. */
  prompt?: boolean;
  /** Codex asked with `request_user_input_async`, which does not block its turn. */
  asyncQuestion?: boolean;
}

export const EMBEDDED_SURFACES: readonly AgentSurfaceKind[] = ["chat", "terminal", "editor"];

export const AGENT_ACTIVITY_STATES: readonly AgentActivityState[] = ["working", "waiting", "done"];

export const SYSTEM_SOUNDS = [
  "Basso",
  "Blow",
  "Bottle",
  "Frog",
  "Funk",
  "Glass",
  "Hero",
  "Morse",
  "Ping",
  "Pop",
  "Purr",
  "Sosumi",
  "Submarine",
  "Tink",
] as const;

export type AlertCategory = "done" | "waiting";

export interface AgentAlertSettings {
  /** `null` silences the category. */
  sounds: Record<AlertCategory, string | null>;
  notifications: Record<AlertCategory, boolean>;
}

export const DEFAULT_AGENT_ALERTS: AgentAlertSettings = {
  sounds: { done: "Submarine", waiting: "Ping" },
  notifications: { done: false, waiting: true },
};

export function normalizeAgentAlerts(value: unknown): AgentAlertSettings {
  const stored = (value && typeof value === "object" ? value : {}) as Record<string, unknown>;
  const sounds = (stored.sounds ?? {}) as Record<string, unknown>;
  const notifications = (stored.notifications ?? {}) as Record<string, unknown>;
  const sound = (category: AlertCategory) => {
    const chosen = sounds[category];
    if (chosen === null) return null;
    return SYSTEM_SOUNDS.includes(chosen as (typeof SYSTEM_SOUNDS)[number])
      ? (chosen as string)
      : DEFAULT_AGENT_ALERTS.sounds[category];
  };
  const notify = (category: AlertCategory) =>
    typeof notifications[category] === "boolean"
      ? (notifications[category] as boolean)
      : DEFAULT_AGENT_ALERTS.notifications[category];
  return {
    sounds: { done: sound("done"), waiting: sound("waiting") },
    notifications: { done: notify("done"), waiting: notify("waiting") },
  };
}

const isWithin = (path: string, root: string) => {
  const base = root.replace(/\/+$/, "");
  return base !== "" && (path === base || path.startsWith(`${base}/`));
};

/** The task whose worktree contains `cwd`; the deepest worktree wins for nested paths. */
export function taskForCwd(tasks: Task[], cwd: string): Task | undefined {
  let best: { task: Task; depth: number } | undefined;
  for (const task of tasks) {
    for (const member of task.members) {
      if (!isWithin(cwd, member.path)) continue;
      if (!best || member.path.length > best.depth) best = { task, depth: member.path.length };
    }
  }
  return best?.task;
}

/**
 * The activity an event leads to, or `null` when it changes nothing: it is older than the last
 * one, or an open question is still unanswered. Codex keeps working and ends its turn after an
 * async question, but the task needs input until the user replies.
 */
export function nextActivity(
  current: AgentActivity | undefined,
  event: Omit<AgentEvent, "cwd" | "taskId">,
  viewing: boolean
): AgentActivity | null {
  if (current && current.at > event.at) return null;
  if (current?.openQuestion && !event.prompt && !event.asyncQuestion)
    return { ...current, at: event.at };
  const { state, agent, surface, at } = event;
  const unread = state !== "working" && !viewing;
  return { state, agent, surface, unread, at, ...(event.asyncQuestion && { openQuestion: true }) };
}

export function applyAgentEvent(
  activities: AgentActivities,
  taskId: string,
  event: Omit<AgentEvent, "cwd" | "taskId">,
  viewing: boolean
): AgentActivities {
  const next = nextActivity(activities[taskId], event, viewing);
  return next ? { ...activities, [taskId]: next } : activities;
}

/** Only a change into waiting or done deserves a sound or a notification. */
export function alertsFor(
  current: AgentActivity | undefined,
  next: AgentActivity | null
): AlertCategory | null {
  if (!next || next.state === "working") return null;
  const unchanged = current?.state === next.state && current.openQuestion === next.openQuestion;
  return unchanged && current?.openQuestion ? null : next.state;
}

/** Opening a task reads it; a read "done" is just idle, so it is dropped. */
export function markTaskSeen(activities: AgentActivities, taskId: string): AgentActivities {
  const activity = activities[taskId];
  if (!activity || (!activity.unread && activity.state !== "done")) return activities;
  const next = { ...activities };
  if (activity.state === "done") delete next[taskId];
  else next[taskId] = { ...activity, unread: false };
  return next;
}

export function pruneActivities(activities: AgentActivities, tasks: Task[]): AgentActivities {
  const ids = new Set(tasks.map((task) => task.id));
  const stale = Object.keys(activities).filter((id) => !ids.has(id));
  if (stale.length === 0) return activities;
  const next = { ...activities };
  for (const id of stale) delete next[id];
  return next;
}

/**
 * An embedded session that is no longer running can't be working or waiting: it was closed or
 * interrupted before its agent reported the end of the turn. Terminal starts are not broadcast,
 * so only a terminal's exit counts.
 */
export function endStaleSessions(
  activities: AgentActivities,
  sessions: Record<string, TerminalStatus>
): AgentActivities {
  const ended = (taskId: string, surface: AgentSurfaceKind) => {
    const session = sessions[taskId];
    if (session) return session.kind === "exited";
    return surface !== "terminal";
  };
  const stale = Object.entries(activities).filter(
    ([taskId, activity]) =>
      activity.state !== "done" &&
      EMBEDDED_SURFACES.includes(activity.surface) &&
      ended(taskId, activity.surface)
  );
  if (stale.length === 0) return activities;
  const next = { ...activities };
  for (const [taskId] of stale) delete next[taskId];
  return next;
}

/**
 * Embedded chats report their state directly, so their hook events are ignored. A chat that
 * turns idle after being busy has finished its turn.
 */
export function chatActivityEvent(
  previous: Pick<ChatInfo, "status" | "waiting"> | undefined,
  chat: ChatInfo,
  at: number
): Omit<AgentEvent, "cwd" | "taskId"> | "end" | null {
  const base = { agent: chat.agent, surface: "chat" as const, at };
  if (!isLive(chat.status)) return "end";
  if (chat.waiting) return previous?.waiting ? null : { ...base, state: "waiting" };
  if (chat.status.kind === "busy")
    return previous?.status.kind === "busy" && !previous.waiting
      ? null
      : { ...base, state: "working" };
  if (chat.status.kind === "idle" && (previous?.status.kind === "busy" || previous?.waiting))
    return { ...base, state: "done" };
  return null;
}

export function unreadCount(activities: AgentActivities): number {
  return Object.values(activities).filter((activity) => activity.unread).length;
}

/** What a task shows: needs input, agent working, a live but idle session, or an ended one. */
export type TaskIndicator = "input" | "working" | "idle" | "ended";

export function taskIndicator(
  activity: AgentActivity | undefined,
  session: TerminalStatus | undefined
): TaskIndicator | null {
  if (activity?.state === "waiting") return "input";
  if (activity?.state === "working") return "working";
  if (session?.kind === "running" || activity?.unread) return "idle";
  if (session?.kind === "exited") return "ended";
  return null;
}

/** Palette and rollup order: needs input, then idle sessions waiting on you, then working. */
export const INDICATOR_RANK: Record<TaskIndicator, number> = {
  input: 3,
  idle: 2,
  working: 1,
  ended: 0,
};

export function strongestIndicator(indicators: (TaskIndicator | null)[]): TaskIndicator | null {
  return indicators.reduce<TaskIndicator | null>(
    (best, indicator) =>
      indicator && (!best || INDICATOR_RANK[indicator] > INDICATOR_RANK[best]) ? indicator : best,
    null
  );
}

export const INDICATOR_LABELS: Record<TaskIndicator, string> = {
  input: "Agent needs your input",
  working: "Agent working",
  idle: "Agent session active",
  ended: "Agent session ended",
};

export const ACTIVITY_LABELS: Record<AgentActivityState, string> = {
  working: "Working",
  waiting: "Needs input",
  done: "Done",
};

const AGENT_NAMES: Record<AgentId, string> = { claude: "Claude", codex: "Codex" };

export interface NotificationTarget {
  taskId: string;
  surface: AgentSurfaceKind;
  agent: AgentId;
}

export function embeddedSurface(target: NotificationTarget): TaskSurface | null {
  if (target.surface === "chat" || target.surface === "terminal")
    return { kind: target.surface, agent: target.agent };
  if (target.surface === "editor") return { kind: "editor" };
  return null;
}

export function notificationFor(
  task: Task,
  event: Pick<AgentEvent, "state" | "agent" | "surface">
) {
  const subject = task.linearIssueIdentifier
    ? `${task.linearIssueIdentifier} · ${task.linearIssueTitle ?? task.branchName}`
    : task.branchName;
  const title =
    event.state === "waiting"
      ? `${AGENT_NAMES[event.agent]} needs your input`
      : `${AGENT_NAMES[event.agent]} finished`;
  const target: NotificationTarget = {
    taskId: task.id,
    surface: event.surface,
    agent: event.agent,
  };
  return { title, body: subject, target };
}

export type HooksState = "missing" | "outdated" | "installed";

export interface AgentHooksStatus {
  claude: HooksState;
  codex: HooksState;
}

export const agentHooksStatus = () => invoke<AgentHooksStatus>("agent_hooks_status");
export const installAgentHooks = (agent: AgentId) => invoke<void>("agent_hooks_install", { agent });
export const removeAgentHooks = (agent: AgentId) => invoke<void>("agent_hooks_remove", { agent });
export const previewSound = (sound: string) =>
  invoke<void>("agent_attention", { sound, notification: null });
