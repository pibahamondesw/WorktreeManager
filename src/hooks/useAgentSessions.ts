import { useEffect, useState } from "react";
import { onTerminalExit, terminalList, TerminalStatus } from "../services/terminal";
import { editorList, onEditorSession, EditorSession } from "../services/codeEditor";

/**
 * Live agent sessions by task id. Sessions are only created from the task view, so the map is
 * refreshed whenever that view opens or closes; exits arrive as events.
 */
export function useAgentSessions(taskOpen: boolean): Record<string, TerminalStatus> {
  const [sessions, setSessions] = useState<Record<string, TerminalStatus>>({});
  const [editors, setEditors] = useState<Record<string, EditorSession>>({});

  useEffect(() => {
    let stale = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      const stop = await onEditorSession((session) => {
        if (!stale) setEditors((previous) => ({ ...previous, [session.taskId]: session }));
      });
      if (stale) {
        stop();
        return;
      }
      unlisten = stop;
      const list = await editorList();
      if (!stale) setEditors(Object.fromEntries(list.map((session) => [session.taskId, session])));
    })().catch(() => undefined);
    return () => {
      stale = true;
      unlisten?.();
    };
  }, [taskOpen]);

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

  const combined = { ...sessions };
  for (const editor of Object.values(editors)) {
    if (editor.status === "running" || editor.status === "starting")
      combined[editor.taskId] = { kind: "running" };
    else if (editor.status === "exited" && !combined[editor.taskId])
      combined[editor.taskId] = { kind: "exited", code: null };
  }
  return combined;
}
