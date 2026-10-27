import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Button } from "../ui/Button";
import { ChatItemView, WorkGroup } from "./ChatItemView";
import { chatEntries, isMainItem, subagentSteps } from "./chatEntries";
import { Composer } from "./Composer";
import { PendingRequestCard } from "./PendingRequestCard";
import { SessionList } from "./SessionList";
import { TodoPanel } from "./TodoPanel";
import { UsageDialog } from "./UsageDialog";
import { HooksDialog } from "./HooksDialog";
import { HistoryDialog } from "./HistoryDialog";
import { HandoffChoice, HandoffDialog } from "./HandoffDialog";
import { ChangeSummaryCard, ChangesPanel, ChangesScope, LineCounts } from "./ChangesPanel";
import { MarkdownPreviewPanel } from "./MarkdownPreviewPanel";
import { MarkdownPreviewContext, MarkdownPreviewRequest } from "./markdownPreviewContext";
import { isPlanItem } from "./markdownDocument";
import { AgentUsageIndicator } from "../usage/AgentUsageIndicator";
import { useChatSession } from "../../hooks/useChatSession";
import { useChatChanges } from "../../hooks/useChatChanges";
import { SessionStatus } from "../../hooks/useTerminalSession";
import {
  ChatChanges,
  ChatImage,
  ChatItem,
  ChatOpenIntent,
  ChatStatus,
  HistoryAction,
  isLive,
  totals,
} from "../../services/chat";
import { AGENTS } from "../../services/agents";
import { AgentId } from "../../types";

const CHAT_PAGE_SIZE = 50;
const LOAD_EARLIER_THRESHOLD_PX = 400;
const STICK_TO_BOTTOM_THRESHOLD_PX = 40;

interface ChatPaneProps {
  taskId: string;
  agent: AgentId;
  folders: string[];
  onStatusChange?: (status: SessionStatus) => void;
  /** Hands the chat to another agent; the pane remounts for it. */
  onSwitchAgent?: (intent: ChatOpenIntent) => void;
  takeOpenIntent?: () => ChatOpenIntent | null;
}

function sessionStatus(status: ChatStatus): SessionStatus {
  if (status.kind === "starting") return { kind: "connecting" };
  if (status.kind === "exited") return { kind: "exited", code: status.code };
  if (status.kind === "failed") return { kind: "exited", code: null };
  return { kind: "running" };
}

/** Maps the last item of each turn to that turn, so its change summary renders right after it. */
function turnEnds(items: ChatItem[], changes: ChatChanges | null) {
  const turnOf = new Map<string, number>();
  changes?.turns.forEach((turn, index) => turn.items.forEach((id) => turnOf.set(id, index)));
  const ends = new Map<number, number>();
  let current: number | undefined;
  items.forEach((item, index) => {
    if (item.kind !== "user") return;
    const turn = turnOf.get(item.id);
    if (turn === current) return;
    if (current !== undefined) ends.set(index - 1, current);
    current = turn;
  });
  if (current !== undefined) ends.set(items.length - 1, current);
  return ends;
}

export function ChatPane({
  taskId,
  agent,
  folders,
  onStatusChange,
  onSwitchAgent,
  takeOpenIntent,
}: ChatPaneProps) {
  const session = useChatSession(taskId, agent, folders, takeOpenIntent);
  const { snapshot, error, restart, respond } = session;
  const [actionError, setActionError] = useState<string | null>(null);
  const [sessionsOpen, setSessionsOpen] = useState(false);
  const [usageOpen, setUsageOpen] = useState(false);
  const [hooksSession, setHooksSession] = useState<string | null>(null);
  const [handoffTarget, setHandoffTarget] = useState<AgentId | null>(null);
  const sessionKey = `${taskId}:${agent}:${snapshot.generation}`;
  const [historyPending, setHistoryPending] = useState<string | null>(null);
  const [historyItem, setHistoryItem] = useState<{
    item: ChatItem;
    generation: number;
    sessionKey: string;
    action: Exclude<HistoryAction, "conversation">;
  } | null>(null);
  const changes = useChatChanges(taskId, agent, snapshot);
  const [changesScope, setChangesScope] = useState<ChangesScope | null>(null);
  const [preview, setPreview] = useState<MarkdownPreviewRequest | null>(null);
  const previewContext = useMemo(() => ({ folders, open: setPreview }), [folders]);
  const [outgoing, setOutgoing] = useState<{
    text: string;
    generation: number;
    users: number;
    sent: boolean;
  } | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);
  const [windowStartId, setWindowStartId] = useState<string | null>(null);
  const distanceFromBottomRef = useRef<number | null>(null);
  const { status, items, pending, controls } = snapshot;
  const busy = status.kind === "busy";
  const live = isLive(status);
  const mainItems = useMemo(() => items.filter(isMainItem), [items]);
  const steps = useMemo(() => subagentSteps(items), [items]);
  const ends = turnEnds(mainItems, changes);
  const entries = useMemo(() => chatEntries(mainItems), [mainItems]);
  const pinnedStart = windowStartId ? entries.findIndex((entry) => entry.id === windowStartId) : -1;
  const firstVisible =
    pinnedStart >= 0 ? pinnedStart : Math.max(0, entries.length - CHAT_PAGE_SIZE);
  const users = items.filter((item) => item.kind === "user").length;
  const showOutgoing =
    outgoing !== null &&
    outgoing.generation === snapshot.generation &&
    users <= outgoing.users &&
    (!outgoing.sent || busy);
  const conversationFiles = changes?.conversation?.files ?? [];
  const pendingPlans = new Set(
    pending.flatMap((request) =>
      request.format === "markdown" && request.detail ? [request.detail] : []
    )
  );
  const previewRequest = preview?.requestId
    ? pending.find((request) => request.id === preview.requestId)
    : undefined;

  const switchBlocker =
    busy || status.kind === "starting" || pending.length > 0 || historyPending !== null
      ? `Wait for ${AGENTS[agent].label} to finish or stop it before switching agents.`
      : null;

  /** Opens `intent.agent` in place of this agent, unless this chat is in the middle of work. */
  const switchTo = (intent: ChatOpenIntent) => {
    if (!onSwitchAgent) return;
    if (switchBlocker) return setActionError(switchBlocker);
    onSwitchAgent(intent);
  };

  const requestAgent = (target: AgentId) => {
    if (target === agent) return;
    if (switchBlocker) return setActionError(switchBlocker);
    if (items.some((item) => item.kind === "user")) setHandoffTarget(target);
    else switchTo({ agent: target, mode: "fresh" });
  };

  const chooseHandoff = (choice: HandoffChoice) => {
    const target = handoffTarget;
    setHandoffTarget(null);
    if (!target) return;
    switchTo({
      agent: target,
      mode: "fresh",
      handoffFrom: choice === "continue" ? agent : undefined,
    });
  };

  useEffect(() => onStatusChange?.(sessionStatus(status)), [status, onStatusChange]);

  useEffect(() => {
    stickRef.current = true;
    setWindowStartId(null);
  }, [snapshot.generation]);

  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickRef.current) el.scrollTop = el.scrollHeight;
  }, [items, pending, outgoing]);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    const distanceFromBottom = distanceFromBottomRef.current;
    if (!el || distanceFromBottom === null) return;
    distanceFromBottomRef.current = null;
    el.scrollTop = el.scrollHeight - distanceFromBottom;
  }, [firstVisible]);

  const showEarlier = () => {
    const el = scrollRef.current;
    if (!el || firstVisible === 0 || distanceFromBottomRef.current !== null) return;
    distanceFromBottomRef.current = el.scrollHeight - el.scrollTop;
    setWindowStartId(entries[Math.max(0, firstVisible - CHAT_PAGE_SIZE)].id);
  };

  const onScroll = (el: HTMLDivElement) => {
    stickRef.current =
      el.scrollHeight - el.scrollTop - el.clientHeight < STICK_TO_BOTTOM_THRESHOLD_PX;
    if (stickRef.current) setWindowStartId(null);
    else if (el.scrollTop < LOAD_EARLIER_THRESHOLD_PX) showEarlier();
    else if (!windowStartId && entries[firstVisible]) setWindowStartId(entries[firstVisible].id);
  };

  const run = async (action: () => Promise<void>) => {
    setActionError(null);
    try {
      await action();
    } catch (e) {
      setActionError(String(e));
    }
  };

  const send = async (text: string, images: ChatImage[]) => {
    stickRef.current = true;
    const message = { text, generation: snapshot.generation, users, sent: false };
    setOutgoing(message);
    try {
      await session.send(text, images);
      setOutgoing((current) => (current === message ? { ...message, sent: true } : current));
    } catch (e) {
      setOutgoing((current) => (current === message ? null : current));
      throw e;
    }
  };

  const pickHistory = (item: ChatItem, action: HistoryAction) => {
    const sessionKey = `${taskId}:${agent}`;
    if (action === "conversation") {
      setHistoryPending(sessionKey);
      void run(() => session.changeHistory(item.id, action)).finally(() => {
        setHistoryPending((current) => (current === sessionKey ? null : current));
      });
    } else {
      setHistoryItem({ item, action, generation: snapshot.generation, sessionKey });
    }
  };

  return (
    <MarkdownPreviewContext.Provider value={previewContext}>
      <div className="relative flex-1 min-h-0 flex flex-col bg-bg-primary">
        <div className="flex flex-wrap items-center justify-end gap-2 shrink-0 px-3 py-1">
          {conversationFiles.length > 0 && (
            <Button
              variant="ghost"
              onClick={() => setChangesScope("conversation")}
              className="h-7 px-2 text-xs"
            >
              Changes <LineCounts {...totals(conversationFiles)} />
            </Button>
          )}
          <AgentUsageIndicator agent={agent} />
          <Button
            variant="ghost"
            onClick={() => setSessionsOpen(true)}
            className="h-7 px-2 text-xs"
          >
            Conversations
          </Button>
        </div>
        <SessionList
          open={sessionsOpen}
          taskId={taskId}
          agent={agent}
          folders={folders}
          onClose={() => setSessionsOpen(false)}
          onPick={(owner, id) => {
            setSessionsOpen(false);
            if (owner === agent) void session.resume(id);
            else switchTo({ agent: owner, mode: "restart", conversation: id });
          }}
          onNew={() => {
            setSessionsOpen(false);
            void session.startFresh();
          }}
        />
        <div
          ref={scrollRef}
          onScroll={(e) => onScroll(e.currentTarget)}
          className="chat-scroll flex-1 min-h-0 overflow-y-auto"
        >
          <div className="max-w-3xl mx-auto px-4 py-4 flex flex-col gap-3">
            {items.length === 0 && live && (
              <p className="text-sm text-text-muted text-center py-8">
                {status.kind === "starting"
                  ? `Starting ${AGENTS[agent].label}…`
                  : `Ask ${AGENTS[agent].label} to work on this task.`}
              </p>
            )}
            {firstVisible > 0 && (
              <Button
                variant="ghost"
                onClick={showEarlier}
                className="self-center h-7 px-2 text-xs"
              >
                Show earlier messages ({firstVisible})
              </Button>
            )}
            {entries.slice(firstVisible).map((entry) => {
              const turn = ends.get(entry.lastIndex);
              const turnChanges = turn === undefined ? undefined : changes?.turns[turn]?.changes;
              const item = entry.kind === "item" ? entry.item : entry.items[0];
              if (entry.kind === "item" && isPlanItem(item) && pendingPlans.has(item.text)) {
                return null;
              }
              const view =
                entry.kind === "work" ? (
                  <WorkGroup items={entry.items} running={busy && entry === entries.at(-1)} />
                ) : (
                  <ChatItemView
                    item={item}
                    steps={steps}
                    historyDisabled={
                      status.kind !== "idle" ||
                      pending.length > 0 ||
                      historyPending === `${taskId}:${agent}`
                    }
                    onHistory={
                      item.kind === "user" && !item.origin
                        ? (action) => pickHistory(item, action)
                        : undefined
                    }
                  />
                );
              return (
                <Fragment key={entry.id}>
                  {item.origin ? (
                    <div
                      data-origin={item.origin}
                      title={`From the ${AGENTS[item.origin].label} conversation`}
                      className="flex flex-col opacity-70"
                    >
                      {view}
                    </div>
                  ) : (
                    view
                  )}
                  {turn !== undefined && turnChanges && turnChanges.files.length > 0 && (
                    <ChangeSummaryCard changes={turnChanges} onOpen={() => setChangesScope(turn)} />
                  )}
                </Fragment>
              );
            })}
            {pending.map((request, index) => (
              <PendingRequestCard
                key={request.id}
                request={request}
                focused={index === 0}
                onRespond={(response) => run(() => respond(request.id, response))}
              />
            ))}
            {showOutgoing && (
              <div aria-label="Sending message" className="flex flex-col opacity-60">
                <ChatItemView item={{ id: "outgoing", kind: "user", text: outgoing.text }} />
              </div>
            )}
            {showOutgoing && !outgoing.sent ? (
              <p role="status" className="text-xs text-text-muted animate-pulse">
                Preparing…
              </p>
            ) : (
              busy &&
              pending.length === 0 && (
                <p className="text-xs text-text-muted animate-pulse">
                  {AGENTS[agent].label} is working…
                </p>
              )
            )}
            {!live && (
              <div role="status" className="flex flex-col items-center gap-3 py-6">
                <span className="text-sm text-text-secondary select-text whitespace-pre-wrap text-center">
                  {error ??
                    (status.kind === "failed"
                      ? status.message
                      : `Session ended${status.kind === "exited" && status.code !== null ? ` (exit ${status.code})` : ""}`)}
                </span>
                <Button onClick={() => void restart()} className="h-8 text-xs">
                  Restart chat
                </Button>
              </div>
            )}
          </div>
        </div>
        <div className="border-t border-border px-4 py-3">
          <TodoPanel todos={controls.todos} />
          {actionError && (
            <p role="alert" className="max-w-3xl mx-auto mb-1.5 text-xs text-danger select-text">
              {actionError}
            </p>
          )}
          <Composer
            key={`${taskId}:${agent}`}
            memoryKey={taskId}
            agent={agent}
            status={status}
            controls={controls}
            folders={folders}
            actions={{
              send,
              interrupt: session.interrupt,
              configure: session.configure,
              compact: session.compact,
              review: session.review,
              startFresh: session.startFresh,
              showUsage: () => setUsageOpen(true),
              showHooks: () => setHooksSession(sessionKey),
              switchAgent: onSwitchAgent ? requestAgent : undefined,
            }}
            onError={setActionError}
          />
        </div>
        {changesScope !== null && changes && (
          <ChangesPanel
            taskId={taskId}
            agent={agent}
            changes={changes}
            prompts={Object.fromEntries(
              items.filter((item) => item.kind === "user").map((item) => [item.id, item.text])
            )}
            scope={changesScope}
            onClose={() => setChangesScope(null)}
          />
        )}
        {preview && (
          <MarkdownPreviewPanel
            key={JSON.stringify(preview.source)}
            request={preview}
            folders={folders}
            onClose={() => setPreview(null)}
            footer={
              previewRequest?.kind === "approval" &&
              previewRequest.decisions.map((decision, index) => (
                <Button
                  key={decision.id}
                  variant={index === 0 ? "primary" : "secondary"}
                  className="h-7 px-3 text-xs"
                  onClick={() => {
                    setPreview(null);
                    void run(() => respond(previewRequest.id, { decision: decision.id }));
                  }}
                >
                  {decision.label}
                </Button>
              ))
            }
          />
        )}
        {hooksSession === sessionKey && agent === "codex" && (
          <HooksDialog
            key={sessionKey}
            hooks={controls.hooks}
            onConfigure={session.configure}
            onClose={() => setHooksSession(null)}
          />
        )}
        {handoffTarget && (
          <HandoffDialog
            from={agent}
            to={handoffTarget}
            onChoose={chooseHandoff}
            onClose={() => setHandoffTarget(null)}
          />
        )}
        <UsageDialog
          open={usageOpen}
          usage={controls.usage}
          planUrl={AGENTS[agent].planUrl}
          onRefresh={session.refreshUsage}
          onClose={() => setUsageOpen(false)}
        />
        {historyItem &&
          historyItem.generation === snapshot.generation &&
          historyItem.sessionKey === `${taskId}:${agent}` && (
            <HistoryDialog
              key={`${historyItem.generation}:${historyItem.item.id}:${historyItem.action}`}
              item={historyItem.item}
              action={historyItem.action}
              disabled={status.kind !== "idle" || pending.length > 0}
              onPreview={() => session.previewRewind(historyItem.item.id)}
              onApply={(action, confirmation) =>
                session.changeHistory(historyItem.item.id, action, confirmation)
              }
              onClose={() => setHistoryItem(null)}
            />
          )}
      </div>
    </MarkdownPreviewContext.Provider>
  );
}
