import { useEffect, useState } from "react";
import { onTerminalExit, terminalList, TerminalInfo, TerminalStatus } from "../services/terminal";
import { editorList, onEditorSession, EditorSession } from "../services/codeEditor";
import { chatList, ChatInfo, ChatStatus, isLive, onChatStatus } from "../services/chat";

const chatSessionStatus = (status: ChatStatus): TerminalStatus =>
  isLive(status)
    ? { kind: "running" }
    : { kind: "exited", code: status.kind === "exited" ? status.code : null };

/**
 * Live agent sessions by task id. Sessions are only created from the task view, so the map is
 * refreshed whenever that view opens or closes; exits arrive as events.
 */
export function useAgentSessions(taskOpen: boolean): Record<string, TerminalStatus> {
  const [sessions, setSessions] = useState<TerminalInfo[]>([]);
  const [editors, setEditors] = useState<Record<string, EditorSession>>({});
  const [chats, setChats] = useState<ChatInfo[]>([]);

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
        setSessions(list);
      })
      .catch(() => undefined);
    return () => {
      stale = true;
    };
  }, [taskOpen]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    onTerminalExit(({ taskId, agent, code }) => {
      setSessions((prev) => [
        ...prev.filter((session) => session.taskId !== taskId || session.agent !== agent),
        { taskId, agent, status: { kind: "exited", code } },
      ]);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    let stale = false;
    let unlisten: (() => void) | undefined;
    const upsert = (info: ChatInfo) =>
      setChats((previous) => [
        ...previous.filter((chat) => chat.taskId !== info.taskId || chat.agent !== info.agent),
        info,
      ]);
    void (async () => {
      const stop = await onChatStatus((info) => {
        if (!stale) upsert(info);
      });
      if (stale) {
        stop();
        return;
      }
      unlisten = stop;
      const list = await chatList();
      if (!stale) setChats(list);
    })().catch(() => undefined);
    return () => {
      stale = true;
      unlisten?.();
    };
  }, [taskOpen]);

  const combined: Record<string, TerminalStatus> = {};
  const agentSessions = [
    ...sessions,
    ...chats.map((chat) => ({ ...chat, status: chatSessionStatus(chat.status) })),
  ];
  for (const session of agentSessions) {
    if (combined[session.taskId]?.kind !== "running") combined[session.taskId] = session.status;
  }
  for (const editor of Object.values(editors)) {
    if (editor.status === "running" || editor.status === "starting")
      combined[editor.taskId] = { kind: "running" };
    else if (editor.status === "exited" && !combined[editor.taskId])
      combined[editor.taskId] = { kind: "exited", code: null };
  }
  return combined;
}
