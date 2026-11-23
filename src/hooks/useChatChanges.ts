import { useEffect, useState } from "react";
import { ChatChanges, chatChanges, ChatSnapshot } from "../services/chat";
import { AgentId } from "../types";

/** Turn and conversation changes, refreshed whenever the agent settles; none when not `enabled`. */
export function useChatChanges(
  taskId: string,
  agent: AgentId,
  snapshot: ChatSnapshot,
  tab?: string,
  enabled = true
) {
  const [changes, setChanges] = useState<ChatChanges | null>(null);
  const settled = snapshot.status.kind !== "busy" && snapshot.status.kind !== "starting";
  const latestPrompt = snapshot.items.filter((item) => item.kind === "user").at(-1)?.id;

  useEffect(() => {
    if (!settled || !enabled) return;
    let cancelled = false;
    chatChanges(taskId, agent, tab)
      .then((result) => !cancelled && setChanges(result))
      .catch(() => !cancelled && setChanges(null));
    return () => {
      cancelled = true;
    };
  }, [taskId, agent, tab, snapshot.generation, settled, latestPrompt, enabled]);

  return changes;
}
