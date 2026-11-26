import {
  ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { Button } from "../ui/Button";
import { ChatItemView, QueuedActions, WorkGroup } from "./ChatItemView";
import { ChatFindBar } from "./ChatFindBar";
import { ConversationMinimap } from "./ConversationMinimap";
import { useChatFind } from "./useChatFind";
import { ItemDetailContext } from "./itemDetailContext";
import { chatEntries, isMainItem, subagentSteps } from "./chatEntries";
import { visibleRows } from "./toolVisibility";
import { useToolVisibility } from "./toolVisibilityPreference";
import { Composer, ComposerQuotes, ComposerSeed, QueuedMessageEdit } from "./Composer";
import { ChatQuote, chatQuote } from "./chatQuotes";
import { QuoteSelectionButton } from "./QuoteSelectionButton";
import { useQuoteNavigation } from "./useQuoteNavigation";
import { ApprovalActions } from "./ApprovalActions";
import { ApprovalDraft } from "./approval";
import { PendingRequestCard } from "./PendingRequestCard";
import { TodoPanel } from "./TodoPanel";
import { UsageDialog } from "./UsageDialog";
import { UsageLimitBanner } from "./UsageLimitBanner";
import { HooksDialog } from "./HooksDialog";
import { HistoryDialog } from "./HistoryDialog";
import { HandoffChoice, HandoffDialog } from "./HandoffDialog";
import { ChangeSummaryCard, ChangesPanel, ChangesScope, LineCounts } from "./ChangesPanel";
import { diffReviewMessage } from "./diffComments";
import { MarkdownPreviewPanel, UnsavedFileEdit } from "./MarkdownPreviewPanel";
import { MarkdownPreviewContext, MarkdownPreviewRequest } from "./markdownPreviewContext";
import { useFirstPrompt } from "./firstPromptContext";
import { isPlanItem, MarkdownSource } from "./markdownDocument";
import { AgentUsageIndicator } from "../usage/AgentUsageIndicator";
import { useChatSession } from "../../hooks/useChatSession";
import { useDiffThreads } from "../../hooks/useDiffThreads";
import { useChatChanges } from "../../hooks/useChatChanges";
import { useVirtualRows } from "../../hooks/useVirtualRows";
import { SessionStatus } from "../../hooks/useTerminalSession";
import {
  ChatChanges,
  ChatImage,
  ChatItem,
  ChatOpenIntent,
  ChatResponse,
  ChatStatus,
  PendingRequest,
  HistoryAction,
  isLive,
  limitResetsAt,
  QueuedEdit,
  totals,
} from "../../services/chat";
import { agentPresentation, describeAgent } from "../../services/agents";
import { useAgentCatalog } from "../../hooks/useAgentCatalog";
import { AgentId } from "../../types";

interface ChatPaneProps {
  taskId: string;
  agent: AgentId;
  /** The chat tab the pane shows; the main one when omitted. */
  tab?: string;
  folders: string[];
  /** Where the pane's controls go, such as the tab bar; above the chat when omitted. */
  toolbar?: HTMLElement | null;
  onStatusChange?: (status: SessionStatus) => void;
  /** Hands the chat to another agent; the pane remounts for it. */
  onSwitchAgent?: (intent: ChatOpenIntent) => void;
  takeOpenIntent?: () => ChatOpenIntent | null;
  /** Shows another session of the agent without taking part, such as an automation's. */
  readOnly?: ReadOnlySession;
  /** Opens a side chat about this conversation, asking `question` when there is one. */
  onAskAside?: (question: string) => void;
  /** Added to the composer's draft, such as a side chat's reply; taken once. */
  insertion?: ComposerSeed | null;
  onInserted?: () => void;
  /** The pane is a side chat, whose replies can be added to the main chat's draft. */
  aside?: SideChatRole;
}

interface SideChatRole {
  onBringBack: (reply: string) => void;
}

interface ReadOnlySession {
  sessionId: string;
  /** The session closed: its owner detached every pane before it ended, so none hears it end. */
  ended: boolean;
  /** Takes the composer's place. */
  footer: ReactNode;
}

const CLOSED: ChatStatus = { kind: "exited", code: null };

/** Conversations whose composer was already offered the first prompt, so clearing it sticks. */
const seededConversations = new Set<string>();

/** Queued messages held for editing, by chat; the edit outlives the pane so leaving a task never sends it. */
const openQueuedEdits = new Map<string, { itemId: string; text: string }>();

/** Unsaved file edits from the preview, by chat; they outlive the pane so switching tasks or tabs keeps them. */
const unsavedFileEdits = new Map<string, UnsavedFileEdit>();

const sameSource = (a: MarkdownSource, b: MarkdownSource) =>
  JSON.stringify(a) === JSON.stringify(b);

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
  tab,
  folders,
  toolbar,
  onStatusChange,
  onSwitchAgent,
  takeOpenIntent,
  readOnly,
  onAskAside,
  insertion,
  onInserted,
  aside,
}: ChatPaneProps) {
  const session = useChatSession(taskId, agent, folders, takeOpenIntent, tab, readOnly?.sessionId);
  const chatKey = readOnly
    ? `${taskId}:${agent}#${readOnly.sessionId}`
    : `${taskId}:${agent}${tab ? `@${tab}` : ""}`;
  const catalog = useAgentCatalog();
  const descriptor = describeAgent(agent, catalog);
  const agentLabel = descriptor.label;
  const { snapshot, error, restart, respond } = session;
  const [approvalDrafts, setApprovalDrafts] = useState<Record<string, ApprovalDraft>>({});
  const [responding, setResponding] = useState<string | null>(null);
  const respondingRef = useRef(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [usageOpen, setUsageOpen] = useState(false);
  const [hooksSession, setHooksSession] = useState<string | null>(null);
  const [handoffTarget, setHandoffTarget] = useState<AgentId | null>(null);
  const sessionKey = `${chatKey}:${snapshot.generation}`;
  const [historyPending, setHistoryPending] = useState<string | null>(null);
  const [historyItem, setHistoryItem] = useState<{
    item: ChatItem;
    generation: number;
    sessionKey: string;
    action: Exclude<HistoryAction, "conversation">;
  } | null>(null);
  const changes = useChatChanges(taskId, agent, snapshot, tab, !readOnly);
  const [changesScope, setChangesScope] = useState<ChangesScope | null>(null);
  const [diffThreads, setDiffThreads] = useDiffThreads(chatKey);
  const [preview, setPreview] = useState<(MarkdownPreviewRequest & { generation: number }) | null>(
    () => {
      const kept = unsavedFileEdits.get(chatKey);
      return kept ? { source: kept.source, generation: snapshot.generation } : null;
    }
  );
  const keepUnsavedFileEdit = (edit: UnsavedFileEdit | null) => {
    if (edit) unsavedFileEdits.set(chatKey, edit);
    else unsavedFileEdits.delete(chatKey);
  };
  const keptFileEditOf = (source: MarkdownSource) => {
    const kept = unsavedFileEdits.get(chatKey);
    return kept && sameSource(kept.source, source) ? { base: kept.base, text: kept.text } : null;
  };
  const closePreview = () => {
    unsavedFileEdits.delete(chatKey);
    setPreview(null);
  };
  const previewContext = useMemo(
    () => ({
      folders,
      open: (request: MarkdownPreviewRequest) =>
        setPreview({ ...request, generation: snapshot.generation }),
    }),
    [folders, snapshot.generation]
  );
  const [outgoing, setOutgoing] = useState<{
    text: string;
    generation: number;
    users: number;
    sent: boolean;
  } | null>(null);
  const { items, pending, controls } = snapshot;
  const status = readOnly?.ended ? CLOSED : snapshot.status;
  const busy = status.kind === "busy";
  const live = isLive(status);
  const mainItems = useMemo(() => items.filter(isMainItem), [items]);
  const steps = useMemo(() => subagentSteps(items), [items]);
  const ends = turnEnds(mainItems, changes);
  const entries = useMemo(() => chatEntries(mainItems), [mainItems]);
  const users = items.filter((item) => item.kind === "user").length;
  const showOutgoing =
    outgoing !== null &&
    outgoing.generation === snapshot.generation &&
    users <= outgoing.users &&
    (!outgoing.sent || busy);
  const conversationFiles = changes?.conversation?.files ?? [];
  const pendingPlans = useMemo(
    () =>
      new Set(
        pending.flatMap((request) =>
          request.format === "markdown" && request.detail ? [request.detail] : []
        )
      ),
    [pending]
  );
  const toolVisibility = useToolVisibility();
  const findOpenState = useState(false);
  const [findOpen] = findOpenState;
  const rows = useMemo(
    () =>
      visibleRows(
        entries.filter(
          (entry) =>
            !(entry.kind === "item" && isPlanItem(entry.item) && pendingPlans.has(entry.item.text))
        ),
        findOpen && toolVisibility === "zen" ? "hideAll" : toolVisibility,
        busy
      ),
    [entries, pendingPlans, findOpen, toolVisibility, busy]
  );
  const rowKeys = useMemo(() => rows.map((row) => row.id), [rows]);
  const timeline = useVirtualRows(rowKeys);
  const { stickToBottom } = timeline;
  const earlier = snapshot.earlier ?? 0;
  const firstPrompt = useFirstPrompt();
  const [seed, setSeed] = useState<ComposerSeed | null>(null);
  const startsEmpty =
    !readOnly && !aside && status.kind === "idle" && items.length === 0 && earlier === 0;
  useEffect(() => {
    if (!startsEmpty || seededConversations.has(sessionKey)) return;
    let stale = false;
    firstPrompt(taskId)
      .then((text) => {
        if (stale || seededConversations.has(sessionKey)) return;
        seededConversations.add(sessionKey);
        if (text) setSeed({ key: sessionKey, text });
      })
      .catch(() => undefined);
    return () => {
      stale = true;
    };
  }, [startsEmpty, sessionKey, taskId, firstPrompt]);
  const historyDisabled =
    status.kind !== "idle" || pending.length > 0 || historyPending === chatKey;
  const approvalKey = (id: string) => `${snapshot.generation}:${id}`;
  const approvalDraft = (id: string) => approvalDrafts[approvalKey(id)] ?? {};
  const changeApprovalDraft = (id: string, draft: ApprovalDraft) =>
    setApprovalDrafts((current) => ({ ...current, [approvalKey(id)]: draft }));
  const previewRequest =
    preview?.generation === snapshot.generation && preview.requestId
      ? pending.find((request) => request.id === preview.requestId)
      : undefined;

  const switchBlocker =
    busy || status.kind === "starting" || pending.length > 0 || historyPending !== null
      ? `Wait for ${agentLabel} to finish or stop it before switching agents.`
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
    if (earlier > 0 || items.some((item) => item.kind === "user")) setHandoffTarget(target);
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
    if (insertion) onInserted?.();
  }, [insertion, onInserted]);

  const latestReply = aside
    ? [...mainItems].reverse().find((item) => item.kind === "assistant" && item.text.trim())
    : undefined;

  useLayoutEffect(() => stickToBottom(), [snapshot.generation, stickToBottom]);

  const { loadEarlier } = session;
  useEffect(() => {
    if (timeline.nearTop && earlier > 0) loadEarlier();
  }, [timeline.nearTop, earlier, loadEarlier]);
  const find = useChatFind(rows, timeline, earlier, loadEarlier, findOpenState);
  const [quotes, setQuotes] = useState<ChatQuote[]>([]);
  const showQuote = useQuoteNavigation(timeline);
  const composerQuotes: ComposerQuotes = {
    items: quotes,
    change: setQuotes,
    canShow: (quote) => rowKeys.includes(quote.itemId),
    show: showQuote,
  };
  const addQuote = (itemId: string, text: string) => {
    const origin = mainItems.find((item) => item.id === itemId)?.origin;
    const author = origin && describeAgent(origin, catalog).label;
    setQuotes((current) => [...current, chatQuote(itemId, text, author)]);
  };

  const run = useCallback(async (action: () => Promise<void>) => {
    setActionError(null);
    try {
      await action();
    } catch (e) {
      setActionError(String(e));
    }
  }, []);

  const canSteer = descriptor.capabilities.steer;
  const { steerQueued, unqueue, holdQueued, releaseQueued, changeHistory } = session;
  const [held, setHeld] = useState(() => openQueuedEdits.get(chatKey) ?? null);
  const editedId =
    held && items.some((item) => item.id === held.itemId && item.status === "queued")
      ? held.itemId
      : null;
  const editQueued = useCallback(
    (itemId: string) =>
      void run(async () => {
        const edit = { itemId, text: await holdQueued(itemId) };
        openQueuedEdits.set(chatKey, edit);
        setHeld(edit);
      }),
    [run, holdQueued, chatKey]
  );
  const queuedActions = useMemo<QueuedActions>(
    () => ({
      steer: canSteer ? (itemId) => void run(() => steerQueued(itemId)) : undefined,
      edit: editedId ? undefined : editQueued,
      remove: (itemId) => void run(() => unqueue(itemId)),
    }),
    [canSteer, run, steerQueued, unqueue, editedId, editQueued]
  );
  const queuedEdit: QueuedMessageEdit | undefined =
    held && editedId
      ? {
          id: editedId,
          text: held.text,
          canSteer,
          keepText: (text: string) => openQueuedEdits.set(chatKey, { itemId: editedId, text }),
          finish: async (edit: QueuedEdit) => {
            await releaseQueued(editedId, edit);
            openQueuedEdits.delete(chatKey);
            setHeld(null);
          },
        }
      : undefined;

  const send = async (text: string, images: ChatImage[]) => {
    stickToBottom();
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

  const answerApproval = async (request: PendingRequest, response?: ChatResponse) => {
    if (respondingRef.current) return;
    respondingRef.current = true;
    setResponding(request.id);
    setActionError(null);
    try {
      if (response) await respond(request.id, response);
      else {
        const plan = approvalDraft(request.id).plan ?? request.detail ?? "";
        setPreview({
          source: { kind: "inline", title: "Approved plan", text: plan },
          generation: snapshot.generation,
        });
        await session.implementPlan(plan);
      }
      setPreview(null);
    } catch (e) {
      if (request.kind === "elicitation") throw e;
      setActionError(String(e));
    } finally {
      respondingRef.current = false;
      setResponding(null);
    }
  };

  const sendDiffComments = async () => {
    const sent = diffThreads;
    await send(diffReviewMessage(sent), []);
    setDiffThreads((current) => current.filter((thread) => !sent.includes(thread)));
    setChangesScope(null);
  };

  const pickHistory = useCallback(
    (item: ChatItem, action: HistoryAction) => {
      if (action === "conversation") {
        setHistoryPending(chatKey);
        void run(() => changeHistory(item.id, action)).finally(() => {
          setHistoryPending((current) => (current === chatKey ? null : current));
        });
      } else {
        setHistoryItem({ item, action, generation: snapshot.generation, sessionKey: chatKey });
      }
    },
    [chatKey, run, changeHistory, snapshot.generation]
  );

  const paneControls = (
    <>
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
      {aside && (
        <Button
          variant="ghost"
          disabled={!latestReply || busy}
          title="Add the latest reply to the main chat's draft"
          onClick={() => latestReply && aside.onBringBack(latestReply.text)}
          className="h-7 px-2 text-xs"
        >
          Add reply to main chat
        </Button>
      )}
    </>
  );

  return (
    <MarkdownPreviewContext.Provider value={previewContext}>
      <div className="relative flex-1 min-h-0 flex flex-col bg-bg-primary">
        {toolbar ? (
          createPortal(paneControls, toolbar)
        ) : (
          <div className="flex flex-wrap items-center justify-end gap-2 shrink-0 px-3 py-1">
            {paneControls}
          </div>
        )}
        {find.open && <ChatFindBar find={find} />}
        <div
          ref={timeline.scrollRef}
          onScroll={timeline.onScroll}
          className="chat-scroll flex-1 min-h-0 overflow-y-auto"
        >
          <div className="max-w-3xl mx-auto px-4 py-4 flex flex-col gap-3">
            <ConversationMinimap rows={rows} timeline={timeline} />
            {items.length === 0 && live && !readOnly && (
              <p className="text-sm text-text-muted text-center py-8">
                {status.kind === "starting"
                  ? `Starting ${agentLabel}…`
                  : aside
                    ? "Ask about the conversation. Nothing here is added to it."
                    : `Ask ${agentLabel} to work on this task.`}
              </p>
            )}
            {earlier > 0 && (
              <p className="text-xs text-text-muted text-center">Loading earlier messages…</p>
            )}
            {rows.length > 0 && (
              <div
                ref={timeline.sectionRef}
                onFocus={timeline.onFocus}
                onBlur={timeline.onBlur}
                className="-mb-3"
              >
                {timeline.before > 0 && <div style={{ height: timeline.before }} />}
                {rows.slice(timeline.start, timeline.end).map((entry) => {
                  const turn = ends.get(entry.lastIndex);
                  const turnChanges =
                    turn === undefined ? undefined : changes?.turns[turn]?.changes;
                  const item = entry.kind === "item" ? entry.item : entry.items[0];
                  const view = (
                    <ItemDetailContext.Provider value={session.expandDetail}>
                      {entry.kind === "work" ? (
                        <WorkGroup
                          items={entry.items}
                          runKey={entry.id}
                          running={busy && entry.id === entries.at(-1)?.id}
                        />
                      ) : (
                        <ChatItemView
                          item={item}
                          steps={item.kind === "agent" ? steps : undefined}
                          historyDisabled={historyDisabled}
                          onHistory={
                            !readOnly && !aside && item.kind === "user" && !item.origin
                              ? pickHistory
                              : undefined
                          }
                          queue={!readOnly && item.status === "queued" ? queuedActions : undefined}
                          editing={item.id === editedId}
                        />
                      )}
                    </ItemDetailContext.Provider>
                  );
                  const content =
                    entry.kind === "item" && item.kind === "assistant" ? (
                      <div data-quote-source={item.id} className="contents">
                        {view}
                      </div>
                    ) : (
                      view
                    );
                  return (
                    <div
                      key={entry.id}
                      ref={timeline.rowRef}
                      data-row-key={entry.id}
                      className="flex flex-col gap-3 pb-3"
                    >
                      {item.origin ? (
                        <div
                          data-origin={item.origin}
                          title={`From the ${describeAgent(item.origin, catalog).label} conversation`}
                          className="flex flex-col opacity-70"
                        >
                          {content}
                        </div>
                      ) : (
                        content
                      )}
                      {turn !== undefined && turnChanges && turnChanges.files.length > 0 && (
                        <ChangeSummaryCard
                          changes={turnChanges}
                          onOpen={() => setChangesScope(turn)}
                        />
                      )}
                    </div>
                  );
                })}
                {timeline.after > 0 && <div style={{ height: timeline.after }} />}
              </div>
            )}
            {!readOnly &&
              pending.map((request, index) => (
                <PendingRequestCard
                  key={`${sessionKey}:${request.id}`}
                  request={request}
                  focused={index === 0}
                  draft={approvalDraft(request.id)}
                  onDraftChange={(draft) => changeApprovalDraft(request.id, draft)}
                  disabled={responding !== null}
                  onRespond={(response) => answerApproval(request, response)}
                  onNewConversation={() => answerApproval(request)}
                />
              ))}
            {showOutgoing && (
              <div aria-label="Sending message" className="flex flex-col opacity-60">
                <ChatItemView item={{ id: "outgoing", kind: "user", text: outgoing.text }} />
              </div>
            )}
            {showOutgoing && !outgoing.sent ? (
              <p role="status" className="text-xs text-text-muted motion-pulse-steps">
                Preparing…
              </p>
            ) : (
              busy &&
              pending.length === 0 && (
                <p className="text-xs text-text-muted motion-pulse-steps">
                  {agentLabel} is working…
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
                {!readOnly && !aside && (
                  <Button onClick={() => void restart()} className="h-8 text-xs">
                    Restart chat
                  </Button>
                )}
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
          {!readOnly && !aside && (
            <UsageLimitBanner
              taskId={taskId}
              agent={agent}
              agentLabel={agentLabel}
              tab={tab}
              folders={folders}
              resetsAt={limitResetsAt(mainItems)}
              prompts={users}
            />
          )}
          {readOnly ? (
            readOnly.footer
          ) : (
            <Composer
              key={`${taskId}:${agent}`}
              memoryKey={aside ? chatKey : taskId}
              agent={agent}
              status={status}
              controls={controls}
              folders={folders}
              queuedEdit={queuedEdit}
              seed={insertion ?? seed}
              quotes={composerQuotes}
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
                askAside: onAskAside,
              }}
              onError={setActionError}
            />
          )}
        </div>
        {!readOnly && (status.kind === "idle" || busy) && !editedId && (
          <QuoteSelectionButton
            sectionRef={timeline.sectionRef}
            scrollRef={timeline.scrollRef}
            onQuote={addQuote}
          />
        )}
        {changesScope !== null && changes && (
          <ChangesPanel
            taskId={taskId}
            agent={agent}
            tab={tab}
            changes={changes}
            prompts={Object.fromEntries(
              items.filter((item) => item.kind === "user").map((item) => [item.id, item.text])
            )}
            scope={changesScope}
            threads={diffThreads}
            onThreadsChange={setDiffThreads}
            onSendComments={sendDiffComments}
            onOpenFile={(path, line) =>
              previewContext.open({ source: { kind: "code", path, line } })
            }
            covered={preview !== null}
            onClose={() => setChangesScope(null)}
          />
        )}
        {preview && (
          <MarkdownPreviewPanel
            key={JSON.stringify(preview.source)}
            request={preview}
            folders={folders}
            error={actionError}
            busy={busy}
            initialFileEdit={keptFileEditOf(preview.source)}
            onUnsavedFileEdit={keepUnsavedFileEdit}
            onClose={closePreview}
            editable={
              previewRequest?.kind === "approval" && previewRequest.editablePlan
                ? {
                    text: approvalDraft(previewRequest.id).plan ?? previewRequest.detail ?? "",
                    onChange: (plan) =>
                      changeApprovalDraft(previewRequest.id, {
                        ...approvalDraft(previewRequest.id),
                        plan,
                      }),
                    disabled: responding !== null,
                  }
                : undefined
            }
            footer={
              previewRequest?.kind === "approval" && (
                <ApprovalActions
                  shortcuts={false}
                  request={previewRequest}
                  draft={approvalDraft(previewRequest.id)}
                  onDraftChange={(draft) => changeApprovalDraft(previewRequest.id, draft)}
                  sending={responding !== null}
                  onRespond={(response) => answerApproval(previewRequest, response)}
                  onNewConversation={() => answerApproval(previewRequest)}
                />
              )
            }
          />
        )}
        {hooksSession === sessionKey && (
          <HooksDialog
            key={sessionKey}
            agentLabel={agentLabel}
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
          planUrl={agentPresentation(agent).planUrl}
          onRefresh={session.refreshUsage}
          onClose={() => setUsageOpen(false)}
        />
        {historyItem &&
          historyItem.generation === snapshot.generation &&
          historyItem.sessionKey === chatKey && (
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
