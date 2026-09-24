import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { AgentId } from "../types";

export type TerminalStatus = { kind: "running" } | { kind: "exited"; code: number | null };

export type TerminalEvent = { type: "data"; data: string } | { type: "exit"; code: number | null };

export interface TerminalOpenResult {
  created: boolean;
  status: TerminalStatus;
  replay: string;
}

export interface TerminalInfo {
  taskId: string;
  agent: AgentId;
  status: TerminalStatus;
}

export interface TerminalOpenArgs {
  taskId: string;
  agent: AgentId;
  folders: string[];
  branchName: string;
  cols: number;
  rows: number;
  onEvent: (event: TerminalEvent) => void;
}

export function terminalOpen({ onEvent, ...args }: TerminalOpenArgs): Promise<TerminalOpenResult> {
  const channel = new Channel<TerminalEvent>();
  channel.onmessage = onEvent;
  return invoke<TerminalOpenResult>("terminal_open", { ...args, onEvent: channel });
}

export const terminalWrite = (taskId: string, agent: AgentId, data: string) =>
  invoke<void>("terminal_write", { taskId, agent, data });

export const terminalResize = (taskId: string, agent: AgentId, cols: number, rows: number) =>
  invoke<void>("terminal_resize", { taskId, agent, cols, rows });

export const terminalDetach = (taskId: string, agent: AgentId) =>
  invoke<void>("terminal_detach", { taskId, agent }).catch(() => undefined);

/** Best-effort: delete flows must not fail because no session existed. */
export const terminalStop = (taskId: string, agent: AgentId) =>
  invoke<void>("terminal_stop", { taskId, agent });

export const terminalClose = (taskId: string) =>
  invoke<void>("terminal_close", { taskId }).catch(() => undefined);

export const terminalList = () => invoke<TerminalInfo[]>("terminal_list");

export const onTerminalExit = (
  callback: (payload: { taskId: string; agent: AgentId; code: number | null }) => void
): Promise<UnlistenFn> =>
  listen<{ taskId: string; agent: AgentId; code: number | null }>("terminal-exit", (e) =>
    callback(e.payload)
  );
