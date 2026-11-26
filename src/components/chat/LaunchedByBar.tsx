import { Button } from "../ui/Button";
import { AgentIcon } from "../ui/Icons";
import { agentPresentation } from "../../services/agents";
import { TabConversation } from "../../services/chat";
import { useTaskConversations } from "./taskConversationContext";

/** Names the chat that launched this conversation as its subagent, and leads back to it. */
export function LaunchedByBar({ parent }: { parent: TabConversation }) {
  const conversations = useTaskConversations();
  const task = conversations?.label(parent.taskId);
  if (!conversations || !task) return null;
  return (
    <div className="flex items-center gap-2 shrink-0 px-4 py-1 border-b border-border text-xs text-text-muted">
      <span className="flex-1 min-w-0 flex items-center gap-1.5 truncate select-text">
        ↳ Subagent of
        <AgentIcon
          agent={parent.agent}
          size={12}
          className={`flex-shrink-0 ${agentPresentation(parent.agent).tone.text}`}
        />
        <span className="text-text-secondary">{task}</span>
      </span>
      <Button
        variant="ghost"
        className="h-6 px-2 text-xs"
        onClick={() => conversations.open(parent)}
      >
        Open parent conversation
      </Button>
    </div>
  );
}
