import { createContext, useContext } from "react";
import { ConversationTarget } from "../../services/automationInbox";

/** Closes the Automations modal and opens the conversation in its task; false if the task is gone. */
export type OpenConversation = (target: ConversationTarget) => boolean;

export const OpenConversationContext = createContext<OpenConversation | null>(null);

export const useOpenConversation = () => useContext(OpenConversationContext);
