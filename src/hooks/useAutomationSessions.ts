import { useEffect, useState } from "react";
import { ChatInfo, chatList, onChatStatus } from "../services/chat";
import { AutomationSessionSource, loadSessionSources } from "../services/automationSessions";

export interface AutomationSession {
  chat: ChatInfo;
  /** Null until the runs show which automation and step the session works on. */
  source: AutomationSessionSource | null;
}

/**
 * The automation sessions of a task the app has seen since it opened, ended ones included, in the
 * order they appeared. Their sources are looked up again when a session appears or starts a turn,
 * since a turn may be the next step of a session the steps share.
 */
export function useAutomationSessions(taskId: string) {
  const [chats, setChats] = useState<Record<string, ChatInfo>>({});
  const [loaded, setLoaded] = useState(false);
  const [turns, setTurns] = useState(0);
  const [sources, setSources] = useState<Record<string, AutomationSessionSource>>({});

  useEffect(() => {
    let disposed = false;
    const statuses = new Map<string, string>();
    const record = (chat: ChatInfo) => {
      if (disposed || chat.taskId !== taskId || chat.origin !== "automation") return;
      const previous = statuses.get(chat.sessionId);
      statuses.set(chat.sessionId, chat.status.kind);
      setChats((current) => ({ ...current, [chat.sessionId]: chat }));
      if (previous !== undefined && previous !== "busy" && chat.status.kind === "busy")
        setTurns((current) => current + 1);
    };
    const listening = onChatStatus(record);
    chatList()
      .then((listed) => listed.forEach(record))
      .catch(() => undefined)
      .finally(() => !disposed && setLoaded(true));
    return () => {
      disposed = true;
      void listening.then((stop) => stop()).catch(() => undefined);
    };
  }, [taskId]);

  const ids = Object.keys(chats);
  const lookup = ids.length > 0 ? `${turns}:${[...ids].sort().join(",")}` : "";
  useEffect(() => {
    if (!lookup) return;
    let cancelled = false;
    loadSessionSources(lookup.split(":")[1].split(","))
      .then((found) => !cancelled && setSources(found))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [lookup]);

  const sessions: AutomationSession[] = ids.map((id) => ({
    chat: chats[id],
    source: sources[id] ?? null,
  }));
  return { sessions, loaded };
}
