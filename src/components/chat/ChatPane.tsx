import { useEffect, useRef, useState } from "react";
import { Button } from "../ui/Button";
import { ChatItemView } from "./ChatItemView";
import { PendingRequestCard } from "./PendingRequestCard";
import { useChatSession } from "../../hooks/useChatSession";
import { SessionStatus } from "../../hooks/useTerminalSession";
import { ChatStatus, isLive } from "../../services/chat";
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
  const { snapshot, error, restart, send, interrupt, respond } = useChatSession(
    taskId,
    agent,
    folders
  );
  const [draft, setDraft] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);
  const { status, items, pending } = snapshot;
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

  const submit = async () => {
    const text = draft.trim();
    if (!text || busy || status.kind !== "idle") return;
    stickRef.current = true;
    setActionError(null);
    try {
      await send(text);
      setDraft("");
    } catch (e) {
      setActionError(String(e));
    }
  };

  return (
    <div className="flex-1 min-h-0 flex flex-col bg-bg-primary">
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
          {pending.map((request) => (
            <PendingRequestCard
              key={request.id}
              request={request}
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
      <form
        className="border-t border-border px-4 py-3"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <div className="max-w-3xl mx-auto flex flex-col gap-2">
          {actionError && (
            <p role="alert" className="text-xs text-danger select-text">
              {actionError}
            </p>
          )}
          <div className="flex items-end gap-2">
            <textarea
              aria-label={`Message ${AGENT_LABEL[agent]}`}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  void submit();
                }
              }}
              disabled={!live}
              rows={Math.min(8, Math.max(2, draft.split("\n").length))}
              placeholder={`Message ${AGENT_LABEL[agent]} (⏎ to send, ⇧⏎ for a new line)`}
              className="flex-1 resize-none rounded-lg bg-bg-secondary border border-border px-3 py-2 text-sm text-text-primary placeholder:text-text-muted focus:outline-none focus:border-accent disabled:opacity-50"
              autoFocus
            />
            {busy ? (
              <Button
                type="button"
                variant="secondary"
                className="h-9 text-xs"
                onClick={() => void run(interrupt)}
              >
                Stop
              </Button>
            ) : (
              <Button
                type="submit"
                className="h-9 text-xs"
                disabled={!draft.trim() || status.kind !== "idle"}
              >
                Send
              </Button>
            )}
          </div>
        </div>
      </form>
    </div>
  );
}
