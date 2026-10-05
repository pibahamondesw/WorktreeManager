import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";
import { useScriptSession } from "../../hooks/useTerminalSession";

const ATTACH_RETRY_MS = 300;

/**
 * Live output of a repository setup or teardown script. The PTY is interactive, so prompts such
 * as `doppler login` can be answered here.
 */
export function ScriptOutput({
  taskId,
  session,
  running,
  generation,
}: {
  taskId: string;
  session: string;
  running: boolean;
  generation: number;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const { error, restart } = useScriptSession(containerRef, taskId, session, generation);

  useEffect(() => {
    if (!error || !running) return;
    const retry = window.setTimeout(() => void restart(), ATTACH_RETRY_MS);
    return () => window.clearTimeout(retry);
  }, [error, running, restart]);

  return (
    <div className="relative h-56 rounded-md border border-border bg-bg-primary">
      <div
        ref={containerRef}
        data-testid="script-output"
        className="absolute inset-0 p-2 [&_.xterm]:h-full"
      />
      {error && !running && (
        <p className="absolute inset-0 flex items-center justify-center text-text-muted">{error}</p>
      )}
    </div>
  );
}
