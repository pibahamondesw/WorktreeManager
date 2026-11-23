import { useCallback, useEffect, useRef, useState } from "react";
import { AgentId } from "../types";
import {
  ChatInfo,
  chatList,
  chatSessions,
  ChatTab,
  chatTabs,
  MAIN_TAB,
  onChatStatus,
  onChatTabs,
} from "../services/chat";
import { TaskIndicator } from "../services/agentActivity";

interface ChatTabTitles {
  /** Conversation titles by `agent:conversation`. */
  titles: Record<string, string>;
  /** Agents whose conversations can be renamed. */
  renamable: AgentId[];
}

const conversationKey = (agent: AgentId, conversation: string) => `${agent}:${conversation}`;

/** What a tab's dot shows: only states that ask for a look, not an idle chat. */
export function tabIndicator(chat: ChatInfo | undefined): TaskIndicator | null {
  if (!chat) return null;
  if (chat.hasError || chat.status.kind === "failed") return "error";
  if (chat.waiting) return "input";
  if (chat.status.kind === "busy" || (chat.status.kind === "idle" && chat.backgroundAgents))
    return "working";
  return null;
}

/** Loads conversation titles of the agents with tabs open; one agent failing hides only its own. */
async function loadTitles(
  taskId: string,
  agents: AgentId[],
  folders: string[]
): Promise<ChatTabTitles> {
  const results = await Promise.allSettled(
    agents.map((agent) => chatSessions(taskId, agent, folders))
  );
  const loaded: ChatTabTitles = { titles: {}, renamable: [] };
  results.forEach((result, index) => {
    if (result.status === "rejected" || !result.value) return;
    const agent = agents[index];
    if (result.value.canRename) loaded.renamable.push(agent);
    for (const session of result.value.sessions)
      if (session.title) loaded.titles[conversationKey(agent, session.id)] = session.title;
  });
  return loaded;
}

/**
 * A task's chat tabs with their titles and activity. They reload when the backend reports them
 * changed (another tab opened them, or `wtm task chat`) and when one of the task's chats settles,
 * since that is when a new conversation gets its id and title.
 */
export function useChatTabs(taskId: string, mainAgent: AgentId, folders: string[]) {
  const [tabs, setTabs] = useState<ChatTab[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const [titles, setTitles] = useState<ChatTabTitles>({ titles: {}, renamable: [] });
  const [activity, setActivity] = useState<Record<string, ChatInfo>>({});
  const foldersKey = folders.join("\n");
  const reload = useCallback(() => setRevision((current) => current + 1), []);

  useEffect(() => {
    let cancelled = false;
    chatTabs(taskId, mainAgent, foldersKey.split("\n"))
      .then((loaded) => {
        if (cancelled) return;
        if (!Array.isArray(loaded)) throw new Error("Could not read the chat tabs");
        setError(null);
        setTabs(loaded);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
        setTabs((current) => current ?? [{ id: MAIN_TAB, agent: mainAgent, conversationId: null }]);
      });
    return () => {
      cancelled = true;
    };
  }, [taskId, mainAgent, foldersKey, revision]);

  useEffect(() => {
    let disposed = false;
    const lastStatus = new Map<string, string>();
    const record = (chat: ChatInfo) => {
      if (chat.taskId !== taskId || chat.origin === "automation") return false;
      const tab = chat.tab ?? MAIN_TAB;
      const settled = chat.status.kind === "idle" && lastStatus.get(tab) !== "idle";
      lastStatus.set(tab, chat.status.kind);
      setActivity((current) => ({ ...current, [tab]: chat }));
      return settled;
    };
    const listening = [
      onChatTabs((changed) => changed === taskId && reload()),
      onChatStatus((chat) => {
        if (record(chat)) reload();
      }),
    ];
    chatList()
      .then((chats) => !disposed && chats.forEach(record))
      .catch(() => undefined);
    return () => {
      disposed = true;
      listening.forEach((unlisten) => void unlisten.then((stop) => stop()).catch(() => undefined));
    };
  }, [taskId, reload]);

  const agentsKey = [...new Set(tabs?.map((tab) => tab.agent))].sort().join("\n");
  const titleRequests = useRef({ sent: 0, applied: 0 });
  useEffect(() => {
    titleRequests.current = { sent: 0, applied: 0 };
  }, [taskId, agentsKey, foldersKey]);
  useEffect(() => {
    if (!agentsKey) return;
    const requests = titleRequests.current;
    const request = ++requests.sent;
    void loadTitles(taskId, agentsKey.split("\n"), foldersKey.split("\n")).then((loaded) => {
      if (titleRequests.current !== requests || request < requests.applied) return;
      requests.applied = request;
      setTitles(loaded);
    });
  }, [taskId, agentsKey, foldersKey, revision]);

  const titleOf = (tab: ChatTab) =>
    tab.conversationId ? titles.titles[conversationKey(tab.agent, tab.conversationId)] : undefined;
  const canRename = (tab: ChatTab) => !!tab.conversationId && titles.renamable.includes(tab.agent);
  const indicatorOf = (tab: ChatTab) => {
    const chat = activity[tab.id];
    return chat?.agent === tab.agent ? tabIndicator(chat) : null;
  };

  return {
    tabs,
    setTabs,
    error,
    reload,
    titleOf,
    canRename,
    indicatorOf,
  };
}
