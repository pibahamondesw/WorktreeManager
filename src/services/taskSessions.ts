import { editorClose } from "./codeEditor";
import { terminalClose } from "./terminal";

export async function closeTaskSessions(taskId: string) {
  await Promise.all([editorClose(taskId), terminalClose(taskId)]);
}
