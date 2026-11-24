import { createContext, useContext } from "react";

/** The first message a new conversation in the task is seeded with, or null. */
export type FirstPrompt = (taskId: string) => Promise<string | null>;

export const FirstPromptContext = createContext<FirstPrompt>(() => Promise.resolve(null));

export const useFirstPrompt = () => useContext(FirstPromptContext);
