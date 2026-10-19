import { sessionRegistry } from "../embedded/sessionTransports";

export const closeTaskSessions = (taskId: string) => sessionRegistry.closeAll(taskId);
