import { useEffect, useRef, useState } from "react";
import { Button } from "../ui/Button";
import { ChatItemView } from "./ChatItemView";
import { Composer } from "./Composer";
import { PendingRequestCard } from "./PendingRequestCard";
import { SessionList } from "./SessionList";
import { TodoPanel } from "./TodoPanel";
import { UsageDialog } from "./UsageDialog";
import { useChatSession } from "../../hooks/useChatSession";
import { SessionStatus } from "../../hooks/useTerminalSession";
import { ChatImage, ChatStatus, isLive } from "../../services/chat";
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

export function ChatPane({ taskId, agent, folders, onStatusChange }: ChatPaneProps) {
  const session = useChatSession(taskId, agent, folders);
  const { snapshot, error, restart, respond } = session;
  const [actionError, setActionError] = useState<string | null>(null);
  const [sessionsOpen, setSessionsOpen] = useState(false);
  const [usageOpen, setUsageOpen] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);
  const { status, items, pending, controls } = snapshot;
  const busy = status.kind === "busy";
  const live = isLive(status);

  useEffect(() => onStatusChange?.(sessionStatus(status)), [status, onStatusChange]);

  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickRef.current) el.scrollTop = el.scrollHeight;
  }, [items, pending]);

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
    await session.send(text, images);
  };

  return (
    <div className="relative flex-1 min-h-0 flex flex-col bg-bg-primary">
      <Button
        variant="ghost"
        onClick={() => setSessionsOpen(true)}
        className="absolute top-2 right-3 z-10 h-7 px-2 text-xs"
      >
        Conversations
      </Button>
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
          {items.map((item) => (
            <ChatItemView key={item.id} item={item} />
          ))}
          {pending.map((request, index) => (
            <PendingRequestCard
              key={request.id}
              request={request}
              focused={index === 0}
              onRespond={(response) => run(() => respond(request.id, response))}
            />
          ))}
          {busy && pending.length === 0 && (
            <p className="text-xs text-text-muted animate-pulse">
              {AGENT_LABEL[agent]} is working…
            </p>
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
      <UsageDialog
        open={usageOpen}
        usage={controls.usage}
        onRefresh={session.refreshUsage}
        onClose={() => setUsageOpen(false)}
      />
    </div>
  );
}
