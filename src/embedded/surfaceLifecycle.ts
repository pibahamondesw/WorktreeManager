import { TaskSurface } from "../types";
import { chatList, chatStop, isLive } from "../services/chat";
import { editorClose, editorList, EditorSession } from "../services/codeEditor";
import { openCodeClose, openCodeList } from "../services/opencode";
import { terminalList, terminalStop } from "../services/terminal";

const serverLive = (session: EditorSession) =>
  session.status === "starting" || session.status === "running";

const orEmpty = <Session>(sessions: Promise<Session[]>) => sessions.catch((): Session[] => []);

/**
 * The task's embedded surfaces with a live process, asked fresh from the backend. A list that
 * fails to load counts as empty so the others still report what is live.
 */
export async function listLiveSurfaces(taskId: string): Promise<TaskSurface[]> {
  const [editors, openCodes, chats, terminals] = await Promise.all([
    orEmpty(editorList()),
    orEmpty(openCodeList()),
    orEmpty(chatList()),
    orEmpty(terminalList()),
  ]);
  const ofTask = <Session extends { taskId: string }>(sessions: Session[]) =>
    sessions.filter((session) => session.taskId === taskId);
  return [
    ...ofTask(editors)
      .filter(serverLive)
      .map((): TaskSurface => ({ kind: "editor" })),
    ...ofTask(openCodes)
      .filter(serverLive)
      .map((): TaskSurface => ({ kind: "opencode" })),
    ...ofTask(chats)
      .filter((chat) => isLive(chat.status))
      .map(({ agent }): TaskSurface => ({ kind: "chat", agent })),
    ...ofTask(terminals)
      .filter((terminal) => terminal.status.kind === "running")
      .map(({ agent }): TaskSurface => ({ kind: "terminal", agent })),
  ];
}

/** Ends the surface's process through its normal close path; agent conversations stay resumable. */
export function stopTaskSurface(taskId: string, surface: TaskSurface): Promise<void> {
  if (surface.kind === "editor") return editorClose(taskId);
  if (surface.kind === "opencode") return openCodeClose(taskId);
  if (surface.kind === "chat") return chatStop(taskId, surface.agent);
  if (surface.kind === "terminal") return terminalStop(taskId, surface.agent);
  return Promise.resolve();
}
