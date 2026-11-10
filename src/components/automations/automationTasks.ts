import { createContext, useContext } from "react";
import { Task } from "../../types";

/** The app's tasks, so automation items show their branch and issue. */
export const AutomationTasksContext = createContext<Task[]>([]);

export const useAutomationTasks = () => useContext(AutomationTasksContext);

/** Tasks with an automation item the owner took over, so their cards say so. */
export const TakenOverTasksContext = createContext<ReadonlySet<string>>(new Set());
