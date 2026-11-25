import { useState } from "react";
import { CloseIcon } from "../ui/Icons";
import { ChatPane } from "./ChatPane";
import { ChatOpenIntent } from "../../services/chat";
import { AgentId } from "../../types";

interface SideChatPanelProps {
  taskId: string;
  agent: AgentId;
  tab: string;
  folders: string[];
  takeOpenIntent: () => ChatOpenIntent | null;
  onBringBack: (reply: string) => void;
  onClose: () => void;
}

/** A side chat beside the conversation it asks about; it never writes to that conversation. */
export function SideChatPanel({
  taskId,
  agent,
  tab,
  folders,
  takeOpenIntent,
  onBringBack,
  onClose,
}: SideChatPanelProps) {
  const [toolbar, setToolbar] = useState<HTMLElement | null>(null);
  return (
    <aside
      aria-label="Side chat"
      className="w-[28rem] max-w-[50%] shrink-0 min-h-0 flex flex-col border-l border-border"
    >
      <div className="flex items-center gap-1 shrink-0 h-9 pl-3 pr-2 border-b border-border">
        <span className="text-xs font-medium text-text-secondary">Side chat</span>
        <div ref={setToolbar} className="ml-auto flex items-center gap-1" />
        <button
          type="button"
          aria-label="Close side chat"
          title="Close side chat"
          onClick={onClose}
          className="flex size-6 items-center justify-center rounded text-text-muted hover:bg-bg-hover hover:text-text-primary cursor-pointer"
        >
          <CloseIcon size={12} />
        </button>
      </div>
      <ChatPane
        taskId={taskId}
        agent={agent}
        tab={tab}
        folders={folders}
        toolbar={toolbar}
        takeOpenIntent={takeOpenIntent}
        aside={{ onBringBack }}
      />
    </aside>
  );
}
