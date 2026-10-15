import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { editorPresentation, EditorSession } from "../../services/codeEditor";
import { SessionStatus } from "../../hooks/useTerminalSession";
import { Button } from "../ui/Button";
import { EmbeddedSurface } from "./embeddedSurfaces";

interface EmbeddedSurfacePaneProps {
  surface: EmbeddedSurface;
  taskId: string;
  workspaceId: string;
  folders: string[];
  closing?: boolean;
  onStatusChange: (status: SessionStatus) => void;
}

/** Reserves the task pane for the surface's webview, which the backend draws over it. */
export function EmbeddedSurfacePane({
  surface,
  taskId,
  workspaceId,
  folders,
  closing = false,
  onStatusChange,
}: EmbeddedSurfacePaneProps) {
  const pane = useRef<HTMLDivElement>(null);
  const [attempt, setAttempt] = useState(0);
  const [needsInstall, setNeedsInstall] = useState(false);
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
    const pending = new Map<string, EditorSession>();
    setError(null);
    setEnded(false);
    setNeedsInstall(false);
    onStatusChange({ kind: "connecting" });
    const update = (session: EditorSession) => {
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
      const stop = await surface.onSession((session) => {
        if (session.taskId !== taskId || cancelled) return;
        pending.set(session.generation, session);
        if (generation === session.generation) update(session);
      });
      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;
      if (surface.install && (await surface.install.needed())) {
        if (!cancelled) setNeedsInstall(true);
        return;
      }
      if (cancelled) return;
      const folders = JSON.parse(foldersKey) as string[];
      const session = await surface.open({ taskId, workspaceId, folders });
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
  }, [surface, taskId, workspaceId, foldersKey, attempt, onStatusChange]);

  const restart = async () => {
    try {
      await surface.close(taskId);
      setAttempt((value) => value + 1);
    } catch (error) {
      setError(String(error));
    }
  };

  const install = surface.install;
  return (
    <div
      ref={pane}
      data-testid={surface.testId}
      className="flex-1 min-h-0 min-w-0 flex flex-col items-center justify-center gap-3 p-4 text-sm text-text-muted"
    >
      {needsInstall && install ? (
        <>
          <p>{install.prompt}</p>
          <install.Setup onReady={() => setAttempt((value) => value + 1)} />
        </>
      ) : ended ? (
        <Button onClick={() => void restart()}>{surface.restartLabel}</Button>
      ) : (
        <p>{closing ? `Closing ${surface.name}…` : `Opening ${surface.name}…`}</p>
      )}
      {error && (
        <p role="alert" className="max-w-xl text-danger select-text break-words">
          {error}
        </p>
      )}
    </div>
  );
}
