import { RefObject, useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { AgentId } from "../types";
import {
  terminalDetach,
  terminalOpen,
  terminalResize,
  terminalWrite,
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

/**
 * Mounts an xterm instance into `containerRef` and attaches it to the task's PTY session. The
 * PTY outlives this hook: unmounting only detaches the webview sink.
 */
export function useTerminalSession(
  containerRef: RefObject<HTMLDivElement | null>,
  { taskId, agent, folders, branchName }: TerminalSessionArgs
) {
  const [status, setStatus] = useState<SessionStatus>({ kind: "connecting" });
  const [error, setError] = useState<string | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const attachRef = useRef<() => Promise<void>>(async () => undefined);
  const foldersKey = folders.join("\n");

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
        const result = await terminalOpen({
          taskId,
          agent,
          folders: foldersKey.split("\n"),
          branchName,
          cols: term.cols,
          rows: term.rows,
          onEvent: (event) => {
            if (disposed) return;
            if (event.type === "data") {
              if (attached) term.write(event.data);
              else pending.push(event.data);
            } else {
              setStatus({ kind: "exited", code: event.code });
            }
          },
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
        setError(typeof e === "string" ? e : "Could not start the terminal");
        setStatus({ kind: "exited", code: null });
      }
    };
    attachRef.current = attach;
    void attach();

    const input = term.onData((data) => void terminalWrite(taskId, data));
    const observer = new ResizeObserver(() => {
      safeFit();
      void terminalResize(taskId, term.cols, term.rows);
    });
    observer.observe(el);

    return () => {
      disposed = true;
      observer.disconnect();
      input.dispose();
      void terminalDetach(taskId);
      term.dispose();
      termRef.current = null;
    };
  }, [containerRef, taskId, agent, foldersKey, branchName]);

  const restart = useCallback(async () => {
    termRef.current?.reset();
    await attachRef.current();
  }, []);

  const focus = useCallback(() => termRef.current?.focus(), []);

  return { status, error, restart, focus };
}
