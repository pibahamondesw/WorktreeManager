import { AgentId } from "../types";
import { ChatOpenIntent } from "../services/chat";

/** A request the chat never took (the user cancelled the switch) is dropped after this. */
const REQUEST_TTL_MS = 30_000;

type Listener = (intent: ChatOpenIntent) => void;

const requested = new Map<string, { intent: ChatOpenIntent; at: number }>();
const listeners = new Map<string, Set<Listener>>();

/**
 * Asks a task's chat to open as `intent` says, from outside its pane: the next pane of that agent
 * to open takes it, and an open pane of that agent is told so it can reopen.
 */
export function requestChatIntent(taskId: string, intent: ChatOpenIntent) {
  requested.set(taskId, { intent, at: Date.now() });
  listeners.get(taskId)?.forEach((listener) => listener(intent));
}

export function takeRequestedChatIntent(taskId: string, agent: AgentId): ChatOpenIntent | null {
  const request = requested.get(taskId);
  if (!request || request.intent.agent !== agent) return null;
  requested.delete(taskId);
  return Date.now() - request.at <= REQUEST_TTL_MS ? request.intent : null;
}

export function onChatIntentRequest(taskId: string, listener: Listener): () => void {
  const taskListeners = listeners.get(taskId) ?? new Set<Listener>();
  taskListeners.add(listener);
  listeners.set(taskId, taskListeners);
  return () => {
    taskListeners.delete(listener);
    if (taskListeners.size === 0) listeners.delete(taskId);
  };
}
