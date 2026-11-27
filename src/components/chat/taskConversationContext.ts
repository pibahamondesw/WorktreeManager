import { createContext, useContext } from "react";
import { TabConversation } from "../../services/chat";
import { ConversationTarget } from "../../services/automationInbox";
import { Task } from "../../types";

/** Opens conversations of other tasks' chats, as subagent links point to them. */
export interface TaskConversations {
  /** The task's issue or branch; null when the task is gone. */
  label: (taskId: string) => string | null;
  /** Shows the conversation in its task's chat; false when the task is gone. */
  open: (conversation: TabConversation) => boolean;
}

export function taskConversations(
  tasks: Task[],
  openConversation: (target: ConversationTarget) => boolean
): TaskConversations {
  return {
    label: (taskId) => {
      const task = tasks.find((candidate) => candidate.id === taskId);
      return task ? (task.linearIssueIdentifier ?? task.branchName) : null;
    },
    open: ({ taskId, agent, conversationId }) =>
      openConversation({ taskId, agent, conversationId }),
  };
}

export const TaskConversationsContext = createContext<TaskConversations | null>(null);

export const useTaskConversations = () => useContext(TaskConversationsContext);
