import { chatClose } from "./chat";
import { editorClose } from "./codeEditor";
import { openCodeClose } from "./opencode";
import { terminalClose } from "./terminal";

export async function closeTaskSessions(taskId: string) {
  await Promise.all([
    editorClose(taskId),
    openCodeClose(taskId),
    terminalClose(taskId),
    chatClose(taskId),
  ]);
}
