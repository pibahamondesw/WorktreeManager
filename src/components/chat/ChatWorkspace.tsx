import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "../ui/Button";
import { PlusIcon } from "../ui/Icons";
import { ChatPane } from "./ChatPane";
import { ChatTabBar, ChatTabView } from "./ChatTabBar";
import { SessionList } from "./SessionList";
import { useChatTabs } from "../../hooks/useChatTabs";
import { useAgentCatalog } from "../../hooks/useAgentCatalog";
import { SessionStatus } from "../../hooks/useTerminalSession";
import { describeAgent } from "../../services/agents";
import {
  ChatOpenIntent,
  chatSessionRename,
  chatStop,
  ChatTab,
  chatTabClose,
  chatTabOpen,
  chatTabSetAgent,
  MAIN_TAB,
} from "../../services/chat";
import { onChatIntentRequest, takeRequestedChatIntent } from "../../embedded/chatIntents";
import { AgentId } from "../../types";

interface ChatWorkspaceProps {
  taskId: string;
  /** The task's chat agent: new tabs start with it, and it runs the main tab of a task without tabs. */
  agent: AgentId;
  folders: string[];
  onStatusChange?: (status: SessionStatus) => void;
  /** The active tab runs another agent, so the task's chat agent follows it. */
  onAgentChange?: (agent: AgentId) => void;
}

/** The tab each task showed last, so coming back to a task shows it again. */
const activeTabs = new Map<string, string>();

/** The main tab keeps the commands' legacy shape: it is the task agent's chat without a tab. */
const paneTab = (tab: string) => (tab === MAIN_TAB ? undefined : tab);

/** A task's conversations as tabs: each runs its own agent session, side by side. */
export function ChatWorkspace({
  taskId,
  agent,
  folders,
  onStatusChange,
  onAgentChange,
}: ChatWorkspaceProps) {
  const { tabs, setTabs, error, titleOf, canRename, indicatorOf, reload } = useChatTabs(
    taskId,
    agent,
    folders
  );
  const catalog = useAgentCatalog();
  const [selected, setSelected] = useState(() => activeTabs.get(taskId) ?? null);
  const [revisions, setRevisions] = useState<Record<string, number>>({});
  const [sessionsOpen, setSessionsOpen] = useState(false);
  const [toolbar, setToolbar] = useState<HTMLElement | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const intents = useRef(new Map<string, ChatOpenIntent>());
  const latestTabs = useRef(tabs);
  useEffect(() => {
    latestTabs.current = tabs;
  }, [tabs]);

  const active = tabs?.find((tab) => tab.id === selected) ?? tabs?.[0] ?? null;
  const activeId = active?.id ?? null;
  const activeAgent = active?.agent ?? null;

  const activate = useCallback(
    (id: string) => {
      activeTabs.set(taskId, id);
      setSelected(id);
    },
    [taskId]
  );

  const run = useCallback(async (action: () => Promise<void>) => {
    setActionError(null);
    try {
      await action();
    } catch (e) {
      setActionError(String(e));
    }
  }, []);

  const openTab = useCallback(
    async (tabAgent: AgentId, intent?: ChatOpenIntent) => {
      const opened = await chatTabOpen(taskId, agent, tabAgent);
      if (intent) intents.current.set(opened.id, intent);
      const tab: ChatTab = { ...opened, conversationId: intent?.conversation ?? null };
      setTabs((current) => [...(current ?? []).filter((entry) => entry.id !== tab.id), tab]);
      activate(tab.id);
    },
    [taskId, agent, setTabs, activate]
  );

  /** A conversation already open in a tab is shown there; any other opens in a new tab. */
  const applyIntent = useCallback(
    (intent: ChatOpenIntent) => {
      const open = intent.conversation
        ? latestTabs.current?.find(
            (tab) => tab.agent === intent.agent && tab.conversationId === intent.conversation
          )
        : undefined;
      if (open) activate(open.id);
      else void run(() => openTab(intent.agent, intent));
    },
    [activate, run, openTab]
  );

  const loaded = tabs !== null;
  useEffect(() => {
    if (!loaded) return;
    const requested = takeRequestedChatIntent(taskId);
    if (requested) applyIntent(requested);
    return onChatIntentRequest(taskId, (intent) => {
      takeRequestedChatIntent(taskId);
      applyIntent(intent);
    });
  }, [taskId, loaded, applyIntent]);

  useEffect(() => {
    if (activeAgent && activeAgent !== agent) onAgentChange?.(activeAgent);
  }, [activeAgent, agent, onAgentChange]);

  useEffect(() => {
    if (loaded && !activeId) onStatusChange?.({ kind: "exited", code: null });
  }, [loaded, activeId, onStatusChange]);

  const takeOpenIntent = useCallback(() => {
    if (!activeId) return null;
    const intent = intents.current.get(activeId) ?? null;
    intents.current.delete(activeId);
    return intent;
  }, [activeId]);

  const closeTab = (tab: ChatTab) =>
    run(async () => {
      const index = tabs?.findIndex((entry) => entry.id === tab.id) ?? -1;
      await chatTabClose(taskId, agent, tab.id);
      const rest = (tabs ?? []).filter((entry) => entry.id !== tab.id);
      setTabs(rest);
      if (tab.id === activeId && rest.length > 0)
        activate(rest[Math.min(index, rest.length - 1)].id);
    });

  /** The tab hands its chat to another agent: the current session stops, the next opens. */
  const switchAgent = (tab: ChatTab, intent: ChatOpenIntent) =>
    run(async () => {
      await chatStop(taskId, tab.agent, paneTab(tab.id));
      await chatTabSetAgent(taskId, agent, tab.id, intent.agent);
      intents.current.set(tab.id, intent);
      setTabs((current) =>
        (current ?? []).map((entry) =>
          entry.id === tab.id ? { ...entry, agent: intent.agent, conversationId: null } : entry
        )
      );
      setRevisions((current) => ({ ...current, [tab.id]: (current[tab.id] ?? 0) + 1 }));
    });

  const rename = (tab: ChatTab, title: string) =>
    run(async () => {
      if (!tab.conversationId) return;
      await chatSessionRename(
        taskId,
        tab.agent,
        folders,
        tab.conversationId,
        title,
        paneTab(tab.id)
      );
      reload();
    });

  const views: ChatTabView[] = (tabs ?? []).map((tab) => ({
    ...tab,
    title: titleOf(tab) ?? (tab.conversationId ? "Untitled conversation" : "New conversation"),
    agentLabel: describeAgent(tab.agent, catalog).label,
    canRename: canRename(tab),
    indicator: indicatorOf(tab),
  }));
  const newLabel = `New ${describeAgent(agent, catalog).label} conversation`;
  const openInTabs = (tabs ?? []).flatMap((tab) =>
    tab.conversationId ? [`${tab.agent}:${tab.conversationId}`] : []
  );

  return (
    <div className="flex-1 min-h-0 flex flex-col bg-bg-primary">
      <ChatTabBar
        tabs={views}
        active={activeId}
        newLabel={newLabel}
        onActivate={activate}
        onClose={(tab) => void closeTab(tab)}
        onRename={(tab, title) => void rename(tab, title)}
        onNew={() => void run(() => openTab(agent))}
        onConversations={() => setSessionsOpen(true)}
        toolbarRef={setToolbar}
      />
      {(actionError ?? error) && (
        <p role="alert" className="px-4 py-1.5 text-xs text-danger select-text">
          {actionError ?? error}
        </p>
      )}
      {active ? (
        <ChatPane
          key={`${active.id}:${active.agent}:${revisions[active.id] ?? 0}`}
          taskId={taskId}
          agent={active.agent}
          tab={paneTab(active.id)}
          folders={folders}
          toolbar={toolbar}
          onStatusChange={onStatusChange}
          onSwitchAgent={(intent) => void switchAgent(active, intent)}
          takeOpenIntent={takeOpenIntent}
        />
      ) : (
        loaded && (
          <div className="flex-1 flex flex-col items-center justify-center gap-3">
            <p className="text-sm text-text-muted">No conversations are open.</p>
            <Button
              variant="secondary"
              className="h-8 text-xs"
              onClick={() => void run(() => openTab(agent))}
            >
              <PlusIcon />
              {newLabel}
            </Button>
          </div>
        )
      )}
      <SessionList
        open={sessionsOpen}
        taskId={taskId}
        agent={agent}
        folders={folders}
        openInTabs={openInTabs}
        onClose={() => setSessionsOpen(false)}
        onPick={(owner, id) => {
          setSessionsOpen(false);
          applyIntent({ agent: owner, mode: "restart", conversation: id });
        }}
        onNew={() => {
          setSessionsOpen(false);
          void run(() => openTab(agent));
        }}
      />
    </div>
  );
}
