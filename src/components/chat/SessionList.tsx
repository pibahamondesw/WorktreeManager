import { useCallback, useEffect, useState } from "react";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { Modal } from "../ui/Modal";
import { PlusIcon, SpinnerIcon } from "../ui/Icons";
import {
  chatSessionArchive,
  chatSessionRename,
  chatSessions,
  ChatSessionList,
  ChatSessionSummary,
} from "../../services/chat";
import { AgentId } from "../../types";

interface SessionListProps {
  open: boolean;
  taskId: string;
  agent: AgentId;
  agentLabel: string;
  folders: string[];
  onClose: () => void;
  onPick: (id: string) => void;
  onNew: () => void;
}

function formatUpdated(seconds: number): string {
  return seconds ? new Date(seconds * 1000).toLocaleString() : "";
}

export function SessionList({
  open,
  taskId,
  agent,
  agentLabel,
  folders,
  onClose,
  onPick,
  onNew,
}: SessionListProps) {
  const [list, setList] = useState<ChatSessionList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; title: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const foldersKey = folders.join("\n");

  const load = useCallback(async () => {
    setError(null);
    try {
      setList(await chatSessions(taskId, agent, foldersKey.split("\n")));
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
    const { id, title } = renaming;
    setRenaming(null);
    void act(() => chatSessionRename(taskId, agent, folders, id, title));
  };

  const row = (session: ChatSessionSummary) => (
    <li key={session.id} className="group flex items-center gap-2 px-6 py-2 hover:bg-bg-hover">
      {renaming?.id === session.id ? (
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
            value={renaming.title}
            onChange={(e) => setRenaming({ id: session.id, title: e.target.value })}
            onBlur={() => setRenaming(null)}
          />
        </form>
      ) : (
        <button
          type="button"
          disabled={busy || session.current}
          onClick={() => onPick(session.id)}
          className="flex-1 min-w-0 text-left cursor-pointer disabled:cursor-default"
        >
          <span className="block truncate text-sm text-text-primary">
            {session.title || "Untitled conversation"}
          </span>
          <span className="block text-xs text-text-muted">
            {session.current ? "Current · " : ""}
            {formatUpdated(session.updatedAt)}
          </span>
        </button>
      )}
      {renaming?.id !== session.id && (
        <div className="flex gap-1 opacity-0 group-hover:opacity-100 focus-within:opacity-100">
          {list?.canRename && (
            <Button
              variant="ghost"
              className="h-7 px-2 text-xs"
              disabled={busy}
              onClick={() => setRenaming({ id: session.id, title: session.title })}
            >
              Rename
            </Button>
          )}
          {list?.canArchive && !session.current && (
            <Button
              variant="ghost"
              className="h-7 px-2 text-xs"
              disabled={busy}
              onClick={() => void act(() => chatSessionArchive(taskId, agent, folders, session.id))}
            >
              Archive
            </Button>
          )}
        </div>
      )}
    </li>
  );

  return (
    <Modal open={open} onClose={onClose} title={`${agentLabel} conversations`}>
      <div className="px-6 py-3 border-b border-border">
        <Button variant="secondary" className="h-8 text-xs" onClick={onNew}>
          <PlusIcon />
          New conversation
        </Button>
      </div>
      {error && <p className="px-6 py-2 text-xs text-danger select-text">{error}</p>}
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
