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
  chatExpandDetail,
  chatLoadEarlier,
  chatInterrupt,
  chatOpen,
  chatRespond,
  chatSend,
  chatSteerQueued,
  chatUnqueue,
  chatHoldQueued,
  chatReleaseQueued,
  QueuedEdit,
  ChatResponse,
  ReviewTarget,
  ChatSetting,
  ChatSnapshot,
  ChatOpenIntent,
  ChatOpenMode,
  DetailView,
  EMPTY_CONTROLS,
} from "../services/chat";

interface Attachment {
  sessionId: string;
  generation: number;
}

const CONNECTING: ChatSnapshot = {
  generation: 0,
  status: { kind: "starting" },
  items: [],
  pending: [],
  controls: EMPTY_CONTROLS,
};

/**
 * Attaches to the chat session of a task's tab, the main one when `tab` is omitted, or to the
 * session `sessionId` names, such as an automation's. The session outlives this hook: unmounting
 * only detaches the sink of the session it opened. Each batch of events applies as one update.
 * Batches from an older generation (a session replaced by a restart) are dropped, and batches
 * that race the snapshot are queued and replayed on top of it. The first open follows
 * `takeIntent` when the pane was mounted to resume or continue a conversation.
 */
export function useChatSession(
  taskId: string,
  agent: AgentId,
  folders: string[],
  takeIntent?: () => ChatOpenIntent | null,
  tab?: string,
  sessionId?: string
) {
  const [snapshot, setSnapshot] = useState<ChatSnapshot>(CONNECTING);
  const [error, setError] = useState<string | null>(null);
  const [attachment, setAttachment] = useState<Attachment | null>(null);
  const openRef = useRef<(mode: ChatOpenMode) => Promise<void>>(async () => undefined);
  const foldersKey = folders.join("\n");

  useEffect(() => {
    let disposed = false;
    let generation = 0;
    let attachedSession = "";
    let opening = 0;

    const open = async (mode: ChatOpenMode, intent?: ChatOpenIntent) => {
      if (disposed) throw new Error("The chat pane is no longer active");
      const version = ++opening;
      const early: { generation: number; apply: (s: ChatSnapshot) => ChatSnapshot }[] = [];
      let ready = false;
      setError(null);
      setSnapshot(CONNECTING);
      try {
        const { sessionId: openedSession, ...initial } = await chatOpen({
          taskId,
          agent,
          folders: foldersKey.split("\n"),
          mode,
          conversation: intent?.conversation,
          handoffFrom: intent?.handoffFrom,
          tab,
          sessionId,
          asideOf: intent?.asideOf,
          onEvents: (eventGeneration, events) => {
            if (disposed || version !== opening) return;
            const apply = (s: ChatSnapshot) =>
              events.reduce((next, event) => applyChatEvent(next, event), s);
            if (!ready) early.push({ generation: eventGeneration, apply });
            else if (eventGeneration === generation) setSnapshot(apply);
          },
        });
        if (intent?.prompt)
          void chatSend(taskId, agent, intent.prompt, [], tab).catch(() => undefined);
        if (disposed || version !== opening) {
          throw new Error("The conversation changed before it was ready");
        }
        generation = initial.generation;
        attachedSession = openedSession;
        ready = true;
        setAttachment({ sessionId: attachedSession, generation });
        setSnapshot(
          early
            .filter((entry) => entry.generation === initial.generation)
            .reduce((s, entry) => entry.apply(s), initial)
        );
      } catch (e) {
        if (disposed || version !== opening) throw e;
        setError(typeof e === "string" ? e : "Could not start the chat");
        setSnapshot({ ...CONNECTING, status: { kind: "failed", message: String(e) } });
        throw e;
      }
    };
    openRef.current = open;
    const taken = takeIntent?.();
    const intent = taken?.agent === agent ? taken : undefined;
    void open(intent?.mode ?? "attach", intent).catch(() => undefined);

    return () => {
      disposed = true;
      if (generation) void chatDetach(taskId, agent, generation, attachedSession, tab);
    };
  }, [taskId, agent, tab, foldersKey, takeIntent, sessionId]);

  const restart = useCallback(() => openRef.current("restart").catch(() => undefined), []);
  const startFresh = useCallback(() => openRef.current("fresh").catch(() => undefined), []);
  const implementPlan = useCallback(
    async (plan: string) => {
      await openRef.current("fresh");
      await chatSend(taskId, agent, `Implement the following approved plan:\n\n${plan}`, [], tab);
    },
    [taskId, agent, tab]
  );
  const configure = useCallback(
    (setting: ChatSetting) => chatConfigure(taskId, agent, setting, tab),
    [taskId, agent, tab]
  );
  const compact = useCallback(() => chatCompact(taskId, agent, tab), [taskId, agent, tab]);
  const review = useCallback(
    (target: ReviewTarget) => chatReview(taskId, agent, target, tab),
    [taskId, agent, tab]
  );
  const send = useCallback(
    (text: string, images: ChatImage[]) => chatSend(taskId, agent, text, images, tab),
    [taskId, agent, tab]
  );
  const refreshUsage = useCallback(
    () => chatRefreshUsage(taskId, agent, tab),
    [taskId, agent, tab]
  );
  const interrupt = useCallback(() => chatInterrupt(taskId, agent, tab), [taskId, agent, tab]);
  const steerQueued = useCallback(
    (itemId: string) => chatSteerQueued(taskId, agent, itemId, tab),
    [taskId, agent, tab]
  );
  const unqueue = useCallback(
    (itemId: string) => chatUnqueue(taskId, agent, itemId, tab),
    [taskId, agent, tab]
  );
  const holdQueued = useCallback(
    (itemId: string) => chatHoldQueued(taskId, agent, itemId, tab),
    [taskId, agent, tab]
  );
  const releaseQueued = useCallback(
    (itemId: string, edit: QueuedEdit) => chatReleaseQueued(taskId, agent, itemId, edit, tab),
    [taskId, agent, tab]
  );
  const respond = useCallback(
    (requestId: string, response: ChatResponse) =>
      chatRespond(taskId, agent, requestId, response, snapshot.generation, tab),
    [taskId, agent, tab, snapshot.generation]
  );
  const expandDetail = useCallback(
    (itemId: string, view: DetailView | null) => {
      if (!attachment) return;
      void chatExpandDetail(attachment.sessionId, attachment.generation, itemId, view);
    },
    [attachment]
  );
  const loadEarlier = useCallback(() => {
    if (attachment) void chatLoadEarlier(attachment.sessionId, attachment.generation);
  }, [attachment]);
  const previewRewind = (itemId: string) =>
    chatRewindPreview(taskId, agent, snapshot.generation, itemId, tab);
  const changeHistory = useCallback(
    async (itemId: string, action: HistoryAction, confirmation?: string) => {
      const open = openRef.current;
      await chatHistory(taskId, agent, snapshot.generation, itemId, action, confirmation, tab);
      if (action !== "code") await open("attach");
    },
    [taskId, agent, tab, snapshot.generation]
  );

  return {
    snapshot,
    error,
    restart,
    startFresh,
    implementPlan,
    send,
    interrupt,
    steerQueued,
    unqueue,
    holdQueued,
    releaseQueued,
    respond,
    configure,
    compact,
    review,
    refreshUsage,
    previewRewind,
    changeHistory,
    expandDetail,
    loadEarlier,
  };
}
