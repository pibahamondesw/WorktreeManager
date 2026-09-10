import { useEffect, useRef, useState } from "react";
import {
  editorInstall,
  editorInstallStatus,
  EditorInstallProgress,
} from "../../services/codeEditor";
import { Button } from "../ui/Button";

export function EditorInstallation({ onReady }: { onReady: () => void }) {
  const [progress, setProgress] = useState<EditorInstallProgress | null>(null);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());
  const notified = useRef(false);
  const requested = useRef(false);
  const ready = useRef(onReady);
  useEffect(() => {
    ready.current = onReady;
  }, [onReady]);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const status = await editorInstallStatus();
        if (cancelled) return;
        setProgress(status);
        setNow(Date.now());
        if (status.phase === "ready" && !notified.current && !requested.current) {
          notified.current = true;
          ready.current();
        }
      } catch (error) {
        if (!cancelled) setError(`Cannot read installation status: ${String(error)}`);
      }
      if (!cancelled) timer = setTimeout(() => void poll(), 1000);
    };
    void poll();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, []);

  const install = async () => {
    setStarting(true);
    setError(null);
    notified.current = false;
    requested.current = true;
    try {
      await editorInstall();
      if (!notified.current) {
        notified.current = true;
        ready.current();
      }
    } catch (error) {
      setError(String(error));
    } finally {
      requested.current = false;
      setStarting(false);
    }
  };
  const busy = starting || progress?.active;
  const elapsed =
    progress?.active && progress.startedAt
      ? Math.max(0, Math.floor((now - progress.startedAt) / 1000))
      : null;
  const megabytes = (bytes: number) => `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return (
    <div className="w-full max-w-xl space-y-2 text-sm">
      <Button onClick={() => void install()} disabled={busy} loading={busy}>
        {busy ? "Installing editor…" : "Install editor"}
      </Button>
      <p role="status" className="text-text-muted">
        {progress?.message ||
          (starting
            ? "Starting installation…"
            : "Installs the editor once for this WTM app. Choose extensions inside the editor.")}
        {elapsed !== null && ` (${elapsed}s)`}
      </p>
      {progress?.phase === "downloading" && (
        <div className="space-y-1">
          <progress
            className="w-full"
            aria-label="Editor download"
            max={progress.total || undefined}
            value={progress.total ? progress.downloaded : undefined}
          />
          <p>
            {megabytes(progress.downloaded)}
            {progress.total ? ` / ${megabytes(progress.total)}` : " downloaded"}
          </p>
        </div>
      )}
      {error && (
        <p role="alert" className="text-danger select-text break-words">
          {error}
        </p>
      )}
      {!!progress?.logs.length && (
        <details className="text-xs text-text-muted">
          <summary className="cursor-pointer">Installation log</summary>
          <pre className="mt-2 whitespace-pre-wrap select-text">{progress.logs.join("\n")}</pre>
        </details>
      )}
    </div>
  );
}
