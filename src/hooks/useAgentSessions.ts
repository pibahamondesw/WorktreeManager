import { useEffect, useSyncExternalStore } from "react";
import { SessionRegistry } from "../embedded/sessionRegistry";
import { sessionRegistry } from "../embedded/sessionTransports";
import { TerminalStatus } from "../services/terminal";

type SessionStatuses = Pick<SessionRegistry, "subscribe" | "taskStatuses" | "refresh">;

/**
 * Live agent sessions by task id. Sessions are only created from the task view, so the map is
 * refreshed whenever that view opens or closes; exits arrive as events.
 */
export function useAgentSessions(
  taskOpen: boolean,
  registry: SessionStatuses = sessionRegistry
): Record<string, TerminalStatus> {
  const statuses = useSyncExternalStore(registry.subscribe, registry.taskStatuses);
  useEffect(() => {
    registry.refresh();
  }, [taskOpen, registry]);
  return statuses;
}
