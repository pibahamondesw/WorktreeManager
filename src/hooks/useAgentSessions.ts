import { useEffect, useState } from "react";
import { onTerminalExit, terminalList, TerminalStatus } from "../services/terminal";

/**
 * Live agent sessions by task id. Sessions are only created from the task view, so the map is
 * refreshed whenever that view opens or closes; exits arrive as events.
 */
export function useAgentSessions(taskOpen: boolean): Record<string, TerminalStatus> {
  const [sessions, setSessions] = useState<Record<string, TerminalStatus>>({});

  useEffect(() => {
    let stale = false;
    terminalList()
      .then((list) => {
        if (stale) return;
        setSessions(Object.fromEntries(list.map((s) => [s.taskId, s.status])));
      })
      .catch(() => undefined);
    return () => {
      stale = true;
    };
  }, [taskOpen]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    onTerminalExit(({ taskId, code }) => {
      setSessions((prev) => ({ ...prev, [taskId]: { kind: "exited", code } }));
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return sessions;
}
