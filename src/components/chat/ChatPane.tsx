import { Fragment, useEffect, useRef, useState } from "react";
import { Button } from "../ui/Button";
import { ChatItemView } from "./ChatItemView";
import { Composer } from "./Composer";
import { PendingRequestCard } from "./PendingRequestCard";
import { SessionList } from "./SessionList";
import { TodoPanel } from "./TodoPanel";
import { UsageDialog } from "./UsageDialog";
import { HistoryDialog } from "./HistoryDialog";
import { ChangeSummaryCard, ChangesPanel, ChangesScope, LineCounts } from "./ChangesPanel";
import { CodexUsageIndicator } from "../usage/CodexUsageIndicator";
import { ClaudeUsageIndicator } from "../usage/ClaudeUsageIndicator";
import { useChatSession } from "../../hooks/useChatSession";
import { useChatChanges } from "../../hooks/useChatChanges";
import { SessionStatus } from "../../hooks/useTerminalSession";
import {
  ChatChanges,
  ChatImage,
  ChatItem,
  ChatStatus,
  HistoryAction,
  isLive,
  totals,
} from "../../services/chat";
import { CODEX_PLAN_URL } from "../../services/usage";
import { AgentId } from "../../types";

interface ChatPaneProps {
  taskId: string;
  agent: AgentId;
  folders: string[];
  onStatusChange?: (status: SessionStatus) => void;
}

const AGENT_LABEL: Record<AgentId, string> = { claude: "Claude", codex: "Codex" };

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

export function ChatPane({ taskId, agent, folders, onStatusChange }: ChatPaneProps) {
  const session = useChatSession(taskId, agent, folders);
  const { snapshot, error, restart, respond } = session;
  const [actionError, setActionError] = useState<string | null>(null);
  const [sessionsOpen, setSessionsOpen] = useState(false);
  const [usageOpen, setUsageOpen] = useState(false);
  const [historyPending, setHistoryPending] = useState<string | null>(null);
  const [historyItem, setHistoryItem] = useState<{
    item: ChatItem;
    generation: number;
    sessionKey: string;
    action: Exclude<HistoryAction, "conversation">;
  } | null>(null);
  const changes = useChatChanges(taskId, agent, snapshot);
  const [changesScope, setChangesScope] = useState<ChangesScope | null>(null);
  const [outgoing, setOutgoing] = useState<{
    text: string;
    generation: number;
    users: number;
    sent: boolean;
  } | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);
  const { status, items, pending, controls } = snapshot;
  const busy = status.kind === "busy";
  const live = isLive(status);
  const ends = turnEnds(items, changes);
  const users = items.filter((item) => item.kind === "user").length;
  const showOutgoing =
    outgoing !== null &&
    outgoing.generation === snapshot.generation &&
    users <= outgoing.users &&
    (!outgoing.sent || busy);
  const conversationFiles = changes?.conversation?.files ?? [];

  useEffect(() => onStatusChange?.(sessionStatus(status)), [status, onStatusChange]);

  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickRef.current) el.scrollTop = el.scrollHeight;
  }, [items, pending, outgoing]);

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
        {agent === "claude" ? <ClaudeUsageIndicator /> : <CodexUsageIndicator />}
        <Button variant="ghost" onClick={() => setSessionsOpen(true)} className="h-7 px-2 text-xs">
          Conversations
        </Button>
      </div>
      <SessionList
        open={sessionsOpen}
        taskId={taskId}
        agent={agent}
        agentLabel={AGENT_LABEL[agent]}
        folders={folders}
        onClose={() => setSessionsOpen(false)}
        onPick={(id) => {
          setSessionsOpen(false);
          void session.resume(id);
        }}
        onNew={() => {
          setSessionsOpen(false);
          void session.startFresh();
        }}
      />
      <div
        ref={scrollRef}
        onScroll={(e) => {
          const el = e.currentTarget;
          stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
        className="flex-1 min-h-0 overflow-y-auto"
      >
        <div className="max-w-3xl mx-auto px-4 py-4 flex flex-col gap-3">
          {items.length === 0 && live && (
            <p className="text-sm text-text-muted text-center py-8">
              {status.kind === "starting"
                ? `Starting ${AGENT_LABEL[agent]}…`
                : `Ask ${AGENT_LABEL[agent]} to work on this task.`}
            </p>
          )}
          {items.map((item, index) => {
            const turn = ends.get(index);
            const turnChanges = turn === undefined ? undefined : changes?.turns[turn]?.changes;
            return (
              <Fragment key={item.id}>
                <ChatItemView
                  item={item}
                  historyDisabled={
                    status.kind !== "idle" ||
                    pending.length > 0 ||
                    historyPending === `${taskId}:${agent}`
                  }
                  onHistory={
                    item.kind === "user" ? (action) => pickHistory(item, action) : undefined
                  }
                />
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
                {AGENT_LABEL[agent]} is working…
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
          memoryKey={`${taskId}:${agent}`}
          agentLabel={AGENT_LABEL[agent]}
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
      <UsageDialog
        open={usageOpen}
        usage={controls.usage}
        planUrl={agent === "codex" ? CODEX_PLAN_URL : undefined}
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
  );
}
