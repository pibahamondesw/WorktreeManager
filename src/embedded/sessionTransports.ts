import { ChatInfo, chatClose, chatList, chatStop, isLive, onChatStatus } from "../services/chat";
import { editorClose, editorList, EditorSession, onEditorSession } from "../services/codeEditor";
import {
  onOpenCodeSession,
  openCodeClose,
  openCodeList,
  OpenCodeSession,
} from "../services/opencode";
import {
  onTerminalExit,
  terminalClose,
  terminalList,
  TerminalInfo,
  TerminalStatus,
  terminalStop,
} from "../services/terminal";
import { createSessionRegistry, SessionTransports } from "./sessionRegistry";

interface TaskSessions {
  editor: EditorSession;
  opencode: OpenCodeSession;
  chat: ChatInfo;
  terminal: TerminalInfo;
}

function serverStatus(session: EditorSession): TerminalStatus | null {
  if (
    session.status === "starting" ||
    session.status === "running" ||
    session.status === "suspended"
  )
    return { kind: "running" };
  if (session.status === "exited") return { kind: "exited", code: null };
  return null;
}

const chatStatus = ({ status, hasError }: ChatInfo): TerminalStatus => ({
  ...(isLive(status)
    ? { kind: "running" as const }
    : { kind: "exited" as const, code: status.kind === "exited" ? status.code : null }),
  ...((hasError || status.kind === "failed") && { hasError: true }),
});

const TRANSPORTS: SessionTransports<TaskSessions> = {
  editor: {
    list: editorList,
    onSession: onEditorSession,
    describe: (session) => ({
      taskId: session.taskId,
      surface: { kind: "editor" },
      status: serverStatus(session),
    }),
    stop: (taskId) => editorClose(taskId),
    close: editorClose,
  },
  opencode: {
    list: openCodeList,
    onSession: onOpenCodeSession,
    describe: (session) => ({
      taskId: session.taskId,
      surface: { kind: "opencode" },
      status: serverStatus(session),
    }),
    stop: (taskId) => openCodeClose(taskId),
    close: openCodeClose,
  },
  chat: {
    list: chatList,
    onSession: onChatStatus,
    describe: (chat) => ({
      taskId: chat.taskId,
      surface: { kind: "chat", agent: chat.agent },
      status: chatStatus(chat),
    }),
    stop: (taskId, { agent }) => chatStop(taskId, agent),
    close: chatClose,
  },
  terminal: {
    list: terminalList,
    onSession: (listener) =>
      onTerminalExit(({ taskId, agent, code }) =>
        listener({ taskId, agent, status: { kind: "exited", code } })
      ),
    describe: ({ taskId, agent, status }) => ({
      taskId,
      surface: { kind: "terminal", agent },
      status,
    }),
    stop: (taskId, { agent }) => terminalStop(taskId, agent),
    close: terminalClose,
  },
};

/** Every embedded session of every task, through its own transport. */
export const sessionRegistry = createSessionRegistry(TRANSPORTS);
