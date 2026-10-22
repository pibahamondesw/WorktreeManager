import { useCallback, useEffect, useState } from "react";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { Modal } from "../ui/Modal";
import { PlusIcon, SpinnerIcon } from "../ui/Icons";
import {
  chatSessionArchive,
  chatSessionRename,
  chatSessions,
  ChatSessionSummary,
} from "../../services/chat";
import { AGENTS, installedAgents } from "../../services/agents";
import { AgentId } from "../../types";

interface SessionListProps {
  open: boolean;
  taskId: string;
  /** The agent the chat runs now; only its conversation is current. */
  agent: AgentId;
  folders: string[];
  onClose: () => void;
  onPick: (agent: AgentId, id: string) => void;
  onNew: () => void;
}

interface AgentSession extends ChatSessionSummary {
  agent: AgentId;
  canRename: boolean;
  canArchive: boolean;
}

interface MergedSessions {
  sessions: AgentSession[];
  errors: string[];
}

/**
 * Every installed agent's conversations in this worktree, newest first; one agent failing hides
 * only its own.
 */
async function loadSessions(
  taskId: string,
  agent: AgentId,
  folders: string[]
): Promise<MergedSessions> {
  const agents = await installedAgents(agent);
  const results = await Promise.allSettled(agents.map((id) => chatSessions(taskId, id, folders)));
  const merged: MergedSessions = { sessions: [], errors: [] };
  results.forEach((result, index) => {
    const owner = agents[index];
    if (result.status === "rejected") {
      merged.errors.push(`${AGENTS[owner].label}: ${String(result.reason)}`);
      return;
    }
    const { canRename, canArchive } = result.value;
    for (const session of result.value.sessions)
      merged.sessions.push({
        ...session,
        current: session.current && owner === agent,
        agent: owner,
        canRename,
        canArchive,
      });
  });
  merged.sessions.sort((a, b) => b.updatedAt - a.updatedAt);
  return merged;
}

function formatUpdated(seconds: number): string {
  return seconds ? new Date(seconds * 1000).toLocaleString() : "";
}

export function SessionList({
  open,
  taskId,
  agent,
  folders,
  onClose,
  onPick,
  onNew,
}: SessionListProps) {
  const [list, setList] = useState<MergedSessions | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{
    agent: AgentId;
    id: string;
    title: string;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const foldersKey = folders.join("\n");

  const load = useCallback(async () => {
    setError(null);
    try {
      setList(await loadSessions(taskId, agent, foldersKey.split("\n")));
    } catch (e) {
      setError(String(e));
    }
  }, [taskId, agent, foldersKey]);

  useEffect(() => {
    if (!open) return;
    setList(null);
    setRenaming(null);
    void load();
  }, [open, load]);

  const act = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const saveRename = () => {
    if (!renaming) return;
    const { agent: owner, id, title } = renaming;
    setRenaming(null);
    void act(() => chatSessionRename(taskId, owner, folders, id, title));
  };

  const isRenaming = (session: AgentSession) =>
    renaming?.agent === session.agent && renaming.id === session.id;

  const row = (session: AgentSession) => (
    <li
      key={`${session.agent}:${session.id}`}
      className="group flex items-center gap-2 px-6 py-2 hover:bg-bg-hover"
    >
      {isRenaming(session) ? (
        <form
          className="flex-1"
          onSubmit={(e) => {
            e.preventDefault();
            saveRename();
          }}
        >
          <Input
            autoFocus
            aria-label="Conversation name"
            value={renaming?.title ?? ""}
            onChange={(e) =>
              setRenaming({ agent: session.agent, id: session.id, title: e.target.value })
            }
            onBlur={() => setRenaming(null)}
          />
        </form>
      ) : (
        <button
          type="button"
          disabled={busy || session.current}
          onClick={() => onPick(session.agent, session.id)}
          className="flex-1 min-w-0 text-left cursor-pointer disabled:cursor-default"
        >
          <span className="block truncate text-sm text-text-primary">
            {session.title || "Untitled conversation"}
          </span>
          <span className="flex items-center gap-1.5 text-xs text-text-muted">
            <span
              className={`inline-flex items-center gap-1 rounded px-1 ${AGENTS[session.agent].tone.chip}`}
            >
              <span
                aria-hidden
                className={`h-1.5 w-1.5 rounded-full ${AGENTS[session.agent].tone.dot}`}
              />
              {AGENTS[session.agent].label}
            </span>
            {session.current ? "Current · " : ""}
            {formatUpdated(session.updatedAt)}
          </span>
        </button>
      )}
      {!isRenaming(session) && (
        <div className="flex gap-1 opacity-0 group-hover:opacity-100 focus-within:opacity-100">
          {session.canRename && (
            <Button
              variant="ghost"
              className="h-7 px-2 text-xs"
              disabled={busy}
              onClick={() =>
                setRenaming({ agent: session.agent, id: session.id, title: session.title })
              }
            >
              Rename
            </Button>
          )}
          {session.canArchive && !session.current && (
            <Button
              variant="ghost"
              className="h-7 px-2 text-xs"
              disabled={busy}
              onClick={() =>
                void act(() => chatSessionArchive(taskId, session.agent, folders, session.id))
              }
            >
              Archive
            </Button>
          )}
        </div>
      )}
    </li>
  );

  return (
    <Modal open={open} onClose={onClose} title="Conversations">
      <div className="px-6 py-3 border-b border-border">
        <Button variant="secondary" className="h-8 text-xs" onClick={onNew}>
          <PlusIcon />
          New {AGENTS[agent].label} conversation
        </Button>
      </div>
      {error && <p className="px-6 py-2 text-xs text-danger select-text">{error}</p>}
      {list?.errors.map((message) => (
        <p key={message} className="px-6 py-1 text-xs text-text-muted select-text">
          {message}
        </p>
      ))}
      {!list && !error && (
        <p className="flex items-center gap-2 px-6 py-4 text-xs text-text-muted">
          <SpinnerIcon size={12} /> Loading conversations…
        </p>
      )}
      {list && list.sessions.length === 0 && (
        <p className="px-6 py-4 text-xs text-text-muted">No conversations in this worktree yet.</p>
      )}
      {list && list.sessions.length > 0 && (
        <ul aria-label="Conversations" className="py-1">
          {list.sessions.map(row)}
        </ul>
      )}
    </Modal>
  );
}
