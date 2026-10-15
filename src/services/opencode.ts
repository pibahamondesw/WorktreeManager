import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AgentActivityState } from "./agentActivity";
import { EditorSession } from "./codeEditor";

export type OpenCodeSession = EditorSession;

export interface OpenCodeActivity {
  taskId: string;
  state: AgentActivityState;
  at: number;
}

export const openCodeProbe = () => invoke<boolean>("opencode_probe");
export const openCodeOpen = (taskId: string, folders: string[]) =>
  invoke<OpenCodeSession>("opencode_open", { taskId, folders });
export const openCodeClose = (taskId: string) => invoke<void>("opencode_close", { taskId });
export const openCodeList = () => invoke<OpenCodeSession[]>("opencode_list");
export const onOpenCodeSession = (callback: (session: OpenCodeSession) => void) =>
  listen<OpenCodeSession>("opencode-session", (event) => callback(event.payload));
export const onOpenCodeActivity = (callback: (activity: OpenCodeActivity) => void) =>
  listen<OpenCodeActivity>("opencode-activity", (event) => callback(event.payload));
