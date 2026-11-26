import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "../ui/Button";
import { PlusIcon } from "../ui/Icons";
import { ChatPane } from "./ChatPane";
import { ComposerSeed } from "./Composer";
import { SideChatPanel } from "./SideChatPanel";
import { quotedReply } from "./sideChat";
import { ChatTabBar, ChatTabView } from "./ChatTabBar";
import { SessionList } from "./SessionList";
import { tabIndicator, useChatTabs } from "../../hooks/useChatTabs";
import { AutomationSession, useAutomationSessions } from "../../hooks/useAutomationSessions";
import { AutomationSessionSource, sessionTitle } from "../../services/automationSessions";
import { itemActions, PipelineStep, takeOverItem } from "../../services/automations";
import { AutomationSessionFooter } from "./AutomationSessionFooter";
import { useAgentCatalog } from "../../hooks/useAgentCatalog";
import { SessionStatus } from "../../hooks/useTerminalSession";
import { describeAgent } from "../../services/agents";
import {
  ChatOpenIntent,
  chatSessionRename,
  newSideTab,
  chatStop,
  ChatTab,
  chatTabClose,
  chatTabOpen,
  chatTabSetAgent,
  isLive,
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

/** The side chat open in each task, shown beside the tab it asks about while that tab is active. */
interface SideChat {
  tab: string;
  agent: AgentId;
  source: string;
}

const sideChats = new Map<string, SideChat>();

/** Automation tabs the owner closed, by task; closing one leaves its session running. */
const closedAutomationTabs = new Map<string, ReadonlySet<string>>();

/** The main tab keeps the commands' legacy shape: it is the task agent's chat without a tab. */
const paneTab = (tab: string) => (tab === MAIN_TAB ? undefined : tab);

const automationTabId = (session: AutomationSession) => `automation:${session.chat.sessionId}`;

const stepAgent = (steps: PipelineStep[], id: string | null) => {
  const step = steps.find((candidate) => candidate.id === id);
  return step && "agent" in step.kind ? step.kind.agent.agent.id : null;
};

/**
 * The tab a task opens on: the one it showed last, unless the task has no conversation of its own
 * and an automation is working in it, which it then shows.
 */
function openingTab(
  tabs: ChatTab[],
  automationTabs: AutomationSession[],
  remembered: string | null
): string | null {
  const ownConversation = tabs.some((tab) => tab.conversationId);
  if (ownConversation || automationTabs.length === 0) return remembered ?? tabs[0]?.id ?? null;
  const shown = automationTabs.find((session) => automationTabId(session) === remembered);
  return automationTabId(shown ?? automationTabs[0]);
}

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
  const automation = useAutomationSessions(taskId);
  const catalog = useAgentCatalog();
  const [remembered] = useState(() => activeTabs.get(taskId) ?? null);
  const [selected, setSelected] = useState<string | null>(null);
  const [closedAutomation, setClosedAutomation] = useState(
    () => closedAutomationTabs.get(taskId) ?? new Set<string>()
  );
  const [revisions, setRevisions] = useState<Record<string, number>>({});
  const [sessionsOpen, setSessionsOpen] = useState(false);
  const [toolbar, setToolbar] = useState<HTMLElement | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [side, setSide] = useState(() => sideChats.get(taskId) ?? null);
  const [insertion, setInsertion] = useState<ComposerSeed | null>(null);
  const [editingQueued, setEditingQueued] = useState(false);
  const intents = useRef(new Map<string, ChatOpenIntent>());
  const latestTabs = useRef(tabs);
  useEffect(() => {
    latestTabs.current = tabs;
  }, [tabs]);

  const loaded = tabs !== null && automation.loaded;
  const automationTabs = automation.sessions.filter(
    (session) =>
      !closedAutomation.has(session.chat.sessionId) &&
      (isLive(session.chat.status) || automationTabId(session) === selected)
  );
  const opening = loaded && selected === null ? openingTab(tabs, automationTabs, remembered) : null;
  const current = selected ?? opening;
  const activeAutomation =
    automationTabs.find((session) => automationTabId(session) === current) ?? null;
  const active = activeAutomation
    ? null
    : (tabs?.find((tab) => tab.id === current) ?? (current ? tabs?.[0] : null) ?? null);
  const showingAutomation = activeAutomation !== null;
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
  const showIntent = useCallback(
    async (intent: ChatOpenIntent) => {
      const open = intent.conversation
        ? latestTabs.current?.find(
            (tab) => tab.agent === intent.agent && tab.conversationId === intent.conversation
          )
        : undefined;
      if (open) activate(open.id);
      else await openTab(intent.agent, intent);
    },
    [activate, openTab]
  );
  const applyIntent = useCallback(
    (intent: ChatOpenIntent) => void run(() => showIntent(intent)),
    [run, showIntent]
  );

  useEffect(() => {
    if (opening !== null) activate(opening);
  }, [opening, activate]);

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
    if (loaded && !activeId && !showingAutomation) onStatusChange?.({ kind: "exited", code: null });
  }, [loaded, activeId, showingAutomation, onStatusChange]);

  const takeIntent = useCallback((tab: string | null) => {
    if (!tab) return null;
    const intent = intents.current.get(tab) ?? null;
    intents.current.delete(tab);
    return intent;
  }, []);
  const takeOpenIntent = useCallback(() => takeIntent(activeId), [takeIntent, activeId]);
  const sideTab = side?.tab ?? null;
  const takeSideIntent = useCallback(() => takeIntent(sideTab), [takeIntent, sideTab]);

  const showSide = useCallback(
    (next: SideChat | null) => {
      if (next) sideChats.set(taskId, next);
      else sideChats.delete(taskId);
      setSide(next);
    },
    [taskId]
  );

  const stopSide = useCallback(
    async (closing: SideChat | null) => {
      if (closing) await chatStop(taskId, closing.agent, closing.tab);
    },
    [taskId]
  );

  const closeSide = useCallback(
    (closing: SideChat) => {
      if (sideChats.get(taskId) !== closing) return;
      void run(async () => {
        showSide(null);
        await stopSide(closing);
      });
    },
    [run, showSide, stopSide, taskId]
  );

  useEffect(() => {
    if (side && tabs && !error && !tabs.some((tab) => tab.id === side.source)) closeSide(side);
  }, [side, tabs, error, closeSide]);

  /** Each question opens a fresh side chat, so it knows the conversation as it is now. */
  const askAside = (about: ChatTab, question: string) =>
    void run(async () => {
      const next = { tab: newSideTab(), agent: about.agent, source: about.id };
      intents.current.set(next.tab, {
        agent: next.agent,
        mode: "fresh",
        asideOf: next.source,
        prompt: question.trim() || undefined,
      });
      showSide(next);
      await stopSide(side);
    });

  const bringBack = useCallback(
    (reply: string) =>
      setInsertion({ key: crypto.randomUUID(), text: quotedReply(reply), append: true }),
    []
  );
  const inserted = useCallback(() => setInsertion(null), []);

  const closeTab = (tab: ChatTab) =>
    run(async () => {
      const index = tabs?.findIndex((entry) => entry.id === tab.id) ?? -1;
      await chatTabClose(taskId, agent, tab.id);
      const closingSide = sideChats.get(taskId);
      if (closingSide?.source === tab.id) closeSide(closingSide);
      setTabs((current) => (current ?? []).filter((entry) => entry.id !== tab.id));
      const rest = (tabs ?? []).filter((entry) => entry.id !== tab.id);
      if (tab.id === activeId && rest.length > 0)
        activate(rest[Math.min(index, rest.length - 1)].id);
    });

  const hideAutomationTab = (session: AutomationSession) => {
    const closed = new Set(closedAutomationTabs.get(taskId)).add(session.chat.sessionId);
    closedAutomationTabs.set(taskId, closed);
    setClosedAutomation(closed);
  };

  const closeAutomationTab = (session: AutomationSession) => {
    hideAutomationTab(session);
    if (automationTabId(session) === current && tabs?.[0]) activate(tabs[0].id);
  };

  /** The owner continues the automation's conversation in a chat of their own, in its tab's place. */
  const takeOver = async (session: AutomationSession, source: AutomationSessionSource) => {
    const control = await takeOverItem({
      name: source.automation,
      runId: source.run.id,
      item: source.item.id,
    });
    if (control.status !== "paused")
      throw new Error("The step is still stopping; it pauses once its turn ends");
    if (!control.conversation_id) return closeAutomationTab(session);
    await showIntent({
      agent: stepAgent(source.steps, control.step) ?? session.chat.agent,
      mode: "restart",
      conversation: control.conversation_id,
    });
    hideAutomationTab(session);
  };

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

  const userViews: ChatTabView[] = (tabs ?? []).map((tab) => ({
    ...tab,
    title: titleOf(tab) ?? (tab.conversationId ? "Untitled conversation" : "New conversation"),
    agentLabel: describeAgent(tab.agent, catalog).label,
    canRename: canRename(tab),
    indicator: indicatorOf(tab),
  }));
  const automationViews: ChatTabView[] = automationTabs.map((session) => ({
    id: automationTabId(session),
    agent: session.chat.agent,
    conversationId: null,
    title: session.source ? sessionTitle(session.source) : "Automation",
    agentLabel: `${describeAgent(session.chat.agent, catalog).label} · Automation`,
    canRename: false,
    indicator: tabIndicator(session.chat),
  }));
  const views = [...userViews, ...automationViews];
  const takeOverAction = (session: AutomationSession) => {
    const source = session.source;
    if (
      !source ||
      source.attempt.step !== source.item.phase ||
      !itemActions(source.item, source.steps, source.run).includes("take_over")
    )
      return undefined;
    return () => takeOver(session, source);
  };
  const closeView = (view: ChatTabView) => {
    const session = automationTabs.find((candidate) => automationTabId(candidate) === view.id);
    if (session) closeAutomationTab(session);
    else void closeTab(view);
  };
  const newLabel = `New ${describeAgent(agent, catalog).label} conversation`;
  const openInTabs = (tabs ?? []).flatMap((tab) =>
    tab.conversationId ? [`${tab.agent}:${tab.conversationId}`] : []
  );

  return (
    <div className="flex-1 min-h-0 flex flex-col bg-bg-primary">
      <ChatTabBar
        tabs={views}
        active={activeAutomation ? automationTabId(activeAutomation) : activeId}
        newLabel={newLabel}
        onActivate={activate}
        onClose={closeView}
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
      {activeAutomation ? (
        <ChatPane
          key={automationTabId(activeAutomation)}
          taskId={taskId}
          agent={activeAutomation.chat.agent}
          folders={folders}
          toolbar={toolbar}
          onStatusChange={onStatusChange}
          readOnly={{
            sessionId: activeAutomation.chat.sessionId,
            ended: !isLive(activeAutomation.chat.status),
            footer: (
              <AutomationSessionFooter
                source={activeAutomation.source}
                onTakeOver={takeOverAction(activeAutomation)}
              />
            ),
          }}
        />
      ) : active ? (
        <div className="flex-1 min-h-0 flex">
          <div className="flex-1 min-w-0 flex flex-col">
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
              onAskAside={(question) => askAside(active, question)}
              insertion={side?.source === active.id ? insertion : null}
              onInserted={inserted}
              onQueuedEditChange={setEditingQueued}
            />
          </div>
          {side?.source === active.id && (
            <SideChatPanel
              key={side.tab}
              taskId={taskId}
              agent={side.agent}
              tab={side.tab}
              folders={folders}
              takeOpenIntent={takeSideIntent}
              onBringBack={bringBack}
              bringBackBlocked={editingQueued}
              onClose={() => closeSide(side)}
            />
          )}
        </div>
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
