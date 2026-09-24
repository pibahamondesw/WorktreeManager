import { useCallback, useEffect, useRef, useState } from "react";
import { AgentId } from "../types";
import {
  applyChatEvent,
  chatCompact,
  chatConfigure,
  chatDetach,
  chatInterrupt,
  chatOpen,
  chatRespond,
  chatSend,
  ChatResponse,
  ChatSetting,
  ChatSnapshot,
  EMPTY_CONTROLS,
} from "../services/chat";

type OpenMode = "attach" | "restart" | "fresh";

const CONNECTING: ChatSnapshot = {
  generation: 0,
  status: { kind: "starting" },
  items: [],
  pending: [],
  controls: EMPTY_CONTROLS,
};

/**
 * Attaches to the task agent's chat session. The session outlives this hook: unmounting only
 * detaches the sink. Events from an older generation (a session replaced by a restart) are
 * dropped, and events that race the snapshot are queued and replayed on top of it.
 */
export function useChatSession(taskId: string, agent: AgentId, folders: string[]) {
  const [snapshot, setSnapshot] = useState<ChatSnapshot>(CONNECTING);
  const [error, setError] = useState<string | null>(null);
  const openRef = useRef<(mode: OpenMode) => Promise<void>>(async () => undefined);
  const foldersKey = folders.join("\n");

  useEffect(() => {
    let disposed = false;
    let generation = 0;

    const open = async (mode: OpenMode) => {
      const early: { generation: number; apply: (s: ChatSnapshot) => ChatSnapshot }[] = [];
      let ready = false;
      setError(null);
      setSnapshot(CONNECTING);
      try {
        const initial = await chatOpen({
          taskId,
          agent,
          folders: foldersKey.split("\n"),
          mode,
          onEvent: (eventGeneration, event) => {
            if (disposed) return;
            const apply = (s: ChatSnapshot) => applyChatEvent(s, event);
            if (!ready) early.push({ generation: eventGeneration, apply });
            else if (eventGeneration === generation) setSnapshot(apply);
          },
        });
        if (disposed) return;
        generation = initial.generation;
        ready = true;
        setSnapshot(
          early
            .filter((entry) => entry.generation === initial.generation)
            .reduce((s, entry) => entry.apply(s), initial)
        );
      } catch (e) {
        if (disposed) return;
        setError(typeof e === "string" ? e : "Could not start the chat");
        setSnapshot({ ...CONNECTING, status: { kind: "failed", message: String(e) } });
      }
    };
    openRef.current = open;
    void open("attach");

    return () => {
      disposed = true;
      if (generation) void chatDetach(taskId, agent, generation);
    };
  }, [taskId, agent, foldersKey]);

  const restart = useCallback(() => openRef.current("restart"), []);
  const startFresh = useCallback(() => openRef.current("fresh"), []);
  const configure = useCallback(
    (setting: ChatSetting) => chatConfigure(taskId, agent, setting),
    [taskId, agent]
  );
  const compact = useCallback(() => chatCompact(taskId, agent), [taskId, agent]);
  const send = useCallback((text: string) => chatSend(taskId, agent, text), [taskId, agent]);
  const interrupt = useCallback(() => chatInterrupt(taskId, agent), [taskId, agent]);
  const respond = useCallback(
    (requestId: string, response: ChatResponse) => chatRespond(taskId, agent, requestId, response),
    [taskId, agent]
  );

  return {
    snapshot,
    error,
    restart,
    startFresh,
    send,
    interrupt,
    respond,
    configure,
    compact,
  };
}
