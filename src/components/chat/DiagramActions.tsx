import { ReactNode, useEffect, useRef, useState } from "react";
import { CheckIcon, CopyIcon, DownloadIcon, WarningCircleIcon } from "../ui/Icons";
import { saveDiagramPng } from "./diagramExport";

const SUCCESS_FEEDBACK_MS = 1500;
const ERROR_FEEDBACK_MS = 5000;

type Feedback =
  | { kind: "idle" }
  | { kind: "pending" }
  | { kind: "done" }
  | { kind: "failed"; message: string };

const IDLE: Feedback = { kind: "idle" };

function useActionFeedback(failureMessage: string) {
  const [feedback, setFeedback] = useState<Feedback>(IDLE);
  const resetTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(resetTimer.current), []);

  const show = (next: Feedback, durationMs: number) => {
    setFeedback(next);
    resetTimer.current = setTimeout(() => setFeedback(IDLE), durationMs);
  };

  const run = (action: () => Promise<unknown>) => {
    clearTimeout(resetTimer.current);
    setFeedback({ kind: "pending" });
    action().then(
      (completed) =>
        completed === false ? setFeedback(IDLE) : show({ kind: "done" }, SUCCESS_FEEDBACK_MS),
      () => show({ kind: "failed", message: failureMessage }, ERROR_FEEDBACK_MS)
    );
  };

  return { feedback, run };
}

function FeedbackIcon({ feedback, idle }: { feedback: Feedback; idle: ReactNode }) {
  if (feedback.kind === "done") return <CheckIcon size={14} className="text-success" />;
  if (feedback.kind === "failed") return <WarningCircleIcon size={14} className="text-danger" />;
  return idle;
}

export function DiagramActions({
  source,
  className,
  buttonClassName,
  children,
}: {
  source: string;
  className: string;
  buttonClassName: string;
  children?: ReactNode;
}) {
  const copy = useActionFeedback("Could not copy the diagram source");
  const download = useActionFeedback("Could not save the image");
  const failure = [copy.feedback, download.feedback].find((feedback) => feedback.kind === "failed");
  const active = copy.feedback.kind !== "idle" || download.feedback.kind !== "idle";

  return (
    <div
      role="toolbar"
      aria-label="Diagram actions"
      className={className}
      data-active={active || undefined}
    >
      {failure && (
        <span role="alert" className="px-1 text-xs text-danger select-text">
          {failure.message}
        </span>
      )}
      <button
        type="button"
        className={buttonClassName}
        onClick={() => copy.run(() => navigator.clipboard.writeText(source))}
        aria-label={copy.feedback.kind === "done" ? "Mermaid source copied" : "Copy Mermaid source"}
        title="Copy Mermaid source"
      >
        <FeedbackIcon feedback={copy.feedback} idle={<CopyIcon size={14} />} />
      </button>
      <button
        type="button"
        className={buttonClassName}
        onClick={() => download.run(() => saveDiagramPng(source))}
        disabled={download.feedback.kind === "pending"}
        aria-label={download.feedback.kind === "done" ? "Image saved" : "Download as PNG"}
        title="Download as PNG"
      >
        <FeedbackIcon feedback={download.feedback} idle={<DownloadIcon size={14} />} />
      </button>
      {children}
    </div>
  );
}
