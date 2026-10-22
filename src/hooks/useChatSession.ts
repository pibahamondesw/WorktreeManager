import { useCallback, useEffect, useRef, useState } from "react";
import { AgentId } from "../types";
import {
  ChatImage,
  chatHistory,
  chatRewindPreview,
  HistoryAction,
  applyChatEvent,
  chatCompact,
  chatReview,
  chatRefreshUsage,
  chatConfigure,
  chatDetach,
  chatInterrupt,
  chatOpen,
  chatRespond,
  chatSend,
  ChatResponse,
  ReviewTarget,
  ChatSetting,
  ChatSnapshot,
  ChatOpenIntent,
  ChatOpenMode,
  EMPTY_CONTROLS,
} from "../services/chat";

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
 * dropped, and events that race the snapshot are queued and replayed on top of it. The first
 * open follows `takeIntent` when the pane was mounted to resume or continue a conversation.
 */
export function useChatSession(
  taskId: string,
  agent: AgentId,
  folders: string[],
  takeIntent?: () => ChatOpenIntent | null
) {
  const [snapshot, setSnapshot] = useState<ChatSnapshot>(CONNECTING);
  const [error, setError] = useState<string | null>(null);
  const openRef = useRef<(mode: ChatOpenMode, conversation?: string) => Promise<void>>(
    async () => undefined
  );
  const foldersKey = folders.join("\n");

  useEffect(() => {
    let disposed = false;
    let generation = 0;

    const open = async (mode: ChatOpenMode, conversation?: string, handoffFrom?: AgentId) => {
      if (disposed) return;
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
          conversation,
          handoffFrom,
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
    const taken = takeIntent?.();
    const intent = taken?.agent === agent ? taken : undefined;
    void open(intent?.mode ?? "attach", intent?.conversation, intent?.handoffFrom);

    return () => {
      disposed = true;
      if (generation) void chatDetach(taskId, agent, generation);
    };
  }, [taskId, agent, foldersKey, takeIntent]);

  const restart = useCallback(() => openRef.current("restart"), []);
  const startFresh = useCallback(() => openRef.current("fresh"), []);
  const resume = useCallback(
    (conversation: string) => openRef.current("restart", conversation),
    []
  );
  const configure = useCallback(
    (setting: ChatSetting) => chatConfigure(taskId, agent, setting),
    [taskId, agent]
  );
  const compact = useCallback(() => chatCompact(taskId, agent), [taskId, agent]);
  const review = useCallback(
    (target: ReviewTarget) => chatReview(taskId, agent, target),
    [taskId, agent]
  );
  const send = useCallback(
    (text: string, images: ChatImage[]) => chatSend(taskId, agent, text, images),
    [taskId, agent]
  );
  const refreshUsage = useCallback(() => chatRefreshUsage(taskId, agent), [taskId, agent]);
  const interrupt = useCallback(() => chatInterrupt(taskId, agent), [taskId, agent]);
  const respond = useCallback(
    (requestId: string, response: ChatResponse) => chatRespond(taskId, agent, requestId, response),
    [taskId, agent]
  );
  const previewRewind = (itemId: string) =>
    chatRewindPreview(taskId, agent, snapshot.generation, itemId);
  const changeHistory = async (itemId: string, action: HistoryAction, confirmation?: string) => {
    const open = openRef.current;
    await chatHistory(taskId, agent, snapshot.generation, itemId, action, confirmation);
    if (action !== "code") await open("attach");
  };

  return {
    snapshot,
    error,
    restart,
    startFresh,
    resume,
    send,
    interrupt,
    respond,
    configure,
    compact,
    review,
    refreshUsage,
    previewRewind,
    changeHistory,
  };
}
