import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  editorClose,
  editorInstallStatus,
  editorOpen,
  editorPresentation,
  editorProbe,
  EditorSession,
  onEditorSession,
} from "../../services/codeEditor";
import { SessionStatus } from "../../hooks/useTerminalSession";
import { Button } from "../ui/Button";
import { EditorInstallation } from "./EditorInstallation";

interface EditorPaneProps {
  taskId: string;
  folders: string[];
  onStatusChange: (status: SessionStatus) => void;
}

export function EditorPane({ taskId, folders, onStatusChange }: EditorPaneProps) {
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
      const stop = await onEditorSession((session) => {
        if (session.taskId !== taskId || cancelled) return;
        pending.set(session.generation, session);
        if (generation === session.generation) update(session);
      });
      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;
      const installation = await editorInstallStatus();
      if (cancelled) return;
      if (installation.active) {
        setNeedsInstall(true);
        return;
      }
      const runtime = await editorProbe();
      if (cancelled) return;
      if (!runtime.ready) {
        setNeedsInstall(true);
        return;
      }
      const session = await editorOpen(taskId, JSON.parse(foldersKey) as string[]);
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
      await editorClose(taskId);
      setAttempt((value) => value + 1);
    } catch (error) {
      setError(String(error));
    }
  };

  return (
    <div
      ref={pane}
      data-testid="editor-pane"
      className="flex-1 min-h-0 min-w-0 flex flex-col items-center justify-center gap-3 p-4 text-sm text-text-muted"
    >
      {needsInstall ? (
        <>
          <p>Set up VS Code embedded.</p>
          <EditorInstallation onReady={() => setAttempt((value) => value + 1)} />
        </>
      ) : ended ? (
        <Button onClick={() => void restart()}>Restart editor</Button>
      ) : (
        <p>Opening VS Code…</p>
      )}
      {error && (
        <p role="alert" className="max-w-xl text-danger select-text break-words">
          {error}
        </p>
      )}
    </div>
  );
}
