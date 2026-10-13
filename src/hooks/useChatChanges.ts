import { useEffect, useState } from "react";
import { ChatChanges, chatChanges, ChatSnapshot } from "../services/chat";
import { AgentId } from "../types";

/** Turn and conversation changes, refreshed whenever the agent settles. */
export function useChatChanges(taskId: string, agent: AgentId, snapshot: ChatSnapshot) {
  const [changes, setChanges] = useState<ChatChanges | null>(null);
  const settled = snapshot.status.kind !== "busy" && snapshot.status.kind !== "starting";
  const userItems = snapshot.items.filter((item) => item.kind === "user").length;

  useEffect(() => {
    if (!settled) return;
    let cancelled = false;
    chatChanges(taskId, agent)
      .then((result) => !cancelled && setChanges(result))
      .catch(() => !cancelled && setChanges(null));
    return () => {
      cancelled = true;
    };
  }, [taskId, agent, snapshot.generation, settled, userItems]);

  return changes;
}
