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

/** `agent` names the session: an agent, or a repository script session (`setup:<repoId>`). */
export const terminalWrite = (taskId: string, agent: string, data: string) =>
  invoke<void>("terminal_write", { taskId, agent, data });

export const terminalResize = (taskId: string, agent: string, cols: number, rows: number) =>
  invoke<void>("terminal_resize", { taskId, agent, cols, rows });

export const terminalDetach = (taskId: string, agent: string) =>
  invoke<void>("terminal_detach", { taskId, agent }).catch(() => undefined);

/** Attach to an existing session without starting one. */
export function terminalAttach(
  taskId: string,
  session: string,
  onEvent: (event: TerminalEvent) => void
): Promise<TerminalOpenResult> {
  const channel = new Channel<TerminalEvent>();
  channel.onmessage = onEvent;
  return invoke<TerminalOpenResult>("terminal_attach", { taskId, session, onEvent: channel });
}

/** Exit code of a repository script run in a PTY session; null when it was stopped. */
export const runTaskScript = (args: {
  taskId: string;
  session: string;
  script: string;
  cwd: string;
  env: [string, string][];
  timeoutSecs?: number;
}) => invoke<number | null>("run_task_script", args);

export const scriptCancel = (taskId: string, session: string) =>
  invoke<void>("script_cancel", { taskId, session });

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
