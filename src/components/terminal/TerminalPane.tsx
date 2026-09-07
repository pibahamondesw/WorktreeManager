import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";
import { Button } from "../ui/Button";
import { useKeyboardShortcuts } from "../../hooks/useKeyboardShortcuts";
import { SessionStatus, useTerminalSession } from "../../hooks/useTerminalSession";
import { AgentId } from "../../types";

interface TerminalPaneProps {
  taskId: string;
  agent: AgentId;
  folders: string[];
  branchName: string;
  onStatusChange?: (status: SessionStatus) => void;
}

export function TerminalPane({ taskId, agent, folders, branchName, onStatusChange }: TerminalPaneProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const { status, error, restart } = useTerminalSession(containerRef, {
    taskId,
    agent,
    folders,
    branchName,
  });
  const exited = status.kind === "exited";

  useEffect(() => {
    onStatusChange?.(status);
  }, [status, onStatusChange]);

  useKeyboardShortcuts({ Enter: { handler: () => void restart(), enabled: exited } });

  return (
    <div className="flex-1 min-h-0 relative bg-bg-primary">
      <div
        ref={containerRef}
        data-testid="terminal-container"
        className="absolute inset-0 p-2 [&_.xterm]:h-full"
      />
      {exited && (
        <div
          role="status"
          className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-bg-primary/80 backdrop-blur-sm"
        >
          <span className="text-sm text-text-secondary">
            {error ?? `Session ended${status.code !== null ? ` (exit ${status.code})` : ""}`}
          </span>
          <Button onClick={() => void restart()} className="h-8 text-xs" autoFocus>
            Restart session
            <kbd className="ml-1 text-[0.625rem] opacity-50 font-mono">⏎</kbd>
          </Button>
        </div>
      )}
    </div>
  );
}
