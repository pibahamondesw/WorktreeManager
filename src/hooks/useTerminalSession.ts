import { RefObject, useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { AgentId } from "../types";
import {
  terminalAttach,
  terminalDetach,
  terminalOpen,
  terminalResize,
  terminalWrite,
  TerminalEvent,
  TerminalOpenResult,
  TerminalStatus,
} from "../services/terminal";
import { xtermThemeFromCss } from "../components/terminal/xtermTheme";

export type SessionStatus = { kind: "connecting" } | TerminalStatus;

interface TerminalSessionArgs {
  taskId: string;
  agent: AgentId;
  folders: string[];
  branchName: string;
}

type Connect = (
  cols: number,
  rows: number,
  onEvent: (event: TerminalEvent) => void
) => Promise<TerminalOpenResult>;

/**
 * Mounts an xterm instance into `containerRef` and attaches it to the task's agent PTY session,
 * starting it when needed. The PTY outlives this hook: unmounting only detaches the webview sink.
 */
export function useTerminalSession(
  containerRef: RefObject<HTMLDivElement | null>,
  { taskId, agent, folders, branchName }: TerminalSessionArgs
) {
  const foldersKey = folders.join("\n");
  const connect = useCallback<Connect>(
    (cols, rows, onEvent) =>
      terminalOpen({
        taskId,
        agent,
        folders: foldersKey.split("\n"),
        branchName,
        cols,
        rows,
        onEvent,
      }),
    [taskId, agent, foldersKey, branchName]
  );
  return useXterm(containerRef, taskId, agent, connect, "Could not start the terminal");
}

/** Attaches to a repository script's existing PTY session (`setup:<repoId>`) without starting it. */
export function useScriptSession(
  containerRef: RefObject<HTMLDivElement | null>,
  taskId: string,
  session: string,
  generation: number
) {
  const connect = useCallback<Connect>(
    (cols, rows, onEvent) =>
      terminalAttach(taskId, session, onEvent).then((result) => {
        void terminalResize(taskId, session, cols, rows);
        return result;
      }),
    // A new run replaces the PTY session, so `generation` forces a fresh attach.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [taskId, session, generation]
  );
  return useXterm(containerRef, taskId, session, connect, "No output yet");
}

function useXterm(
  containerRef: RefObject<HTMLDivElement | null>,
  taskId: string,
  session: string,
  connect: Connect,
  failure: string
) {
  const [status, setStatus] = useState<SessionStatus>({ kind: "connecting" });
  const [error, setError] = useState<string | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const attachRef = useRef<() => Promise<void>>(async () => undefined);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const term = new Terminal({
      theme: xtermThemeFromCss(),
      fontFamily: "Menlo, Monaco, monospace",
      fontSize: 12,
      cursorBlink: true,
      scrollback: 5000,
      allowProposedApi: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    const safeFit = () => {
      if (el.clientWidth > 0 && el.clientHeight > 0) fit.fit();
    };
    safeFit();
    termRef.current = term;

    let disposed = false;
    let attached = false;
    const pending: string[] = [];

    const attach = async () => {
      attached = false;
      pending.length = 0;
      setStatus({ kind: "connecting" });
      setError(null);
      try {
        const result = await connect(term.cols, term.rows, (event) => {
          if (disposed) return;
          if (event.type === "data") {
            if (attached) term.write(event.data);
            else pending.push(event.data);
          } else {
            setStatus({ kind: "exited", code: event.code });
          }
        });
        if (disposed) return;
        if (result.replay) term.write(result.replay);
        pending.forEach((chunk) => term.write(chunk));
        pending.length = 0;
        attached = true;
        setStatus(result.status);
        term.focus();
      } catch (e) {
        if (disposed) return;
        setError(typeof e === "string" ? e : failure);
        setStatus({ kind: "exited", code: null });
      }
    };
    attachRef.current = attach;
    void attach();

    const input = term.onData((data) => void terminalWrite(taskId, session, data));
    const observer = new ResizeObserver(() => {
      safeFit();
      void terminalResize(taskId, session, term.cols, term.rows);
    });
    observer.observe(el);

    return () => {
      disposed = true;
      observer.disconnect();
      input.dispose();
      void terminalDetach(taskId, session);
      term.dispose();
      termRef.current = null;
    };
  }, [containerRef, taskId, session, connect, failure]);

  const restart = useCallback(async () => {
    termRef.current?.reset();
    await attachRef.current();
  }, []);

  const focus = useCallback(() => termRef.current?.focus(), []);

  return { status, error, restart, focus };
}
