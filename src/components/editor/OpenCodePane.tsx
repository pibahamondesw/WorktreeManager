import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { editorPresentation } from "../../services/codeEditor";
import {
  onOpenCodeSession,
  openCodeClose,
  openCodeOpen,
  OpenCodeSession,
} from "../../services/opencode";
import { SessionStatus } from "../../hooks/useTerminalSession";
import { Button } from "../ui/Button";

interface OpenCodePaneProps {
  taskId: string;
  folders: string[];
  closing?: boolean;
  onStatusChange: (status: SessionStatus) => void;
}

/** Reserves the task pane for the task's OpenCode webview, which the backend draws over it. */
export function OpenCodePane({
  taskId,
  folders,
  closing = false,
  onStatusChange,
}: OpenCodePaneProps) {
  const pane = useRef<HTMLDivElement>(null);
  const [attempt, setAttempt] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [ended, setEnded] = useState(false);
  const foldersKey = JSON.stringify(folders);

  useLayoutEffect(() => {
    const element = pane.current;
    if (!element) return;
    const bounds = () => {
      const rect = element.getBoundingClientRect();
      return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
    };
    const lease = editorPresentation.attach(taskId, bounds(), setError);
    const resize = () => lease.update(bounds());
    const observer = new ResizeObserver(resize);
    observer.observe(element);
    window.addEventListener("resize", resize);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", resize);
      lease.release();
    };
  }, [taskId, attempt]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let generation: string | undefined;
    const pending = new Map<string, OpenCodeSession>();
    setError(null);
    setEnded(false);
    onStatusChange({ kind: "connecting" });
    const update = (session: OpenCodeSession) => {
      if (
        cancelled ||
        session.taskId !== taskId ||
        (generation && generation !== session.generation)
      )
        return;
      if (session.status === "running") onStatusChange({ kind: "running" });
      if (session.status === "exited" || session.status === "closed") {
        setEnded(true);
        setError(session.error);
        onStatusChange({ kind: "exited", code: null });
      }
    };
    void (async () => {
      const stop = await onOpenCodeSession((session) => {
        if (session.taskId !== taskId || cancelled) return;
        pending.set(session.generation, session);
        if (generation === session.generation) update(session);
      });
      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;
      const session = await openCodeOpen(taskId, JSON.parse(foldersKey) as string[]);
      generation = session.generation;
      update(pending.get(session.generation) ?? session);
    })().catch((error: unknown) => {
      if (!cancelled) {
        setError(String(error));
        setEnded(true);
        onStatusChange({ kind: "exited", code: null });
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [taskId, foldersKey, attempt, onStatusChange]);

  const restart = async () => {
    try {
      await openCodeClose(taskId);
      setAttempt((value) => value + 1);
    } catch (error) {
      setError(String(error));
    }
  };

  return (
    <div
      ref={pane}
      data-testid="opencode-pane"
      className="flex-1 min-h-0 min-w-0 flex flex-col items-center justify-center gap-3 p-4 text-sm text-text-muted"
    >
      {ended ? (
        <Button onClick={() => void restart()}>Restart OpenCode</Button>
      ) : (
        <p>{closing ? "Closing OpenCode…" : "Opening OpenCode…"}</p>
      )}
      {error && (
        <p role="alert" className="max-w-xl text-danger select-text break-words">
          {error}
        </p>
      )}
    </div>
  );
}
