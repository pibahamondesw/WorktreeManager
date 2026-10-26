import { ReactNode } from "react";
import { DeleteOutcome, DeleteRequest } from "../../services/deletion";
import { SCRIPT_SOURCE_LABELS } from "../../services/repoScripts";

interface DeleteOutcomeViewProps {
  outcome: DeleteOutcome;
  deleting: boolean;
  onRetry: (request: DeleteRequest) => void;
  onDismiss: () => void;
  compact?: boolean;
}

const secondaryButton =
  "px-3 py-1 text-xs rounded-md text-text-secondary hover:text-text-primary hover:bg-bg-hover border border-border transition-colors cursor-pointer disabled:opacity-40";
const dangerGhostButton =
  "px-3 py-1 text-xs rounded-md text-danger hover:bg-danger/10 border border-border transition-colors cursor-pointer disabled:opacity-40";
const dangerButton =
  "px-3 py-1 text-xs rounded-md text-white bg-danger hover:bg-danger-hover transition-colors cursor-pointer disabled:opacity-40";

export function DeleteOutcomeView({
  outcome,
  deleting,
  onRetry,
  onDismiss,
  compact,
}: DeleteOutcomeViewProps) {
  const textSize = compact ? "text-xs" : "text-sm";
  const detailSize = compact ? "text-[0.625rem]" : "text-xs";
  const cancel = (label = "Cancel") => (
    <button onClick={onDismiss} disabled={deleting} className={secondaryButton}>
      {label}
    </button>
  );

  if (outcome.kind === "deleted")
    return (
      <div className="space-y-2 select-text">
        <p className={`${textSize} text-text-secondary`}>Deleted with warnings:</p>
        <ul className={`${detailSize} text-warning list-disc pl-4 space-y-1`}>
          {outcome.warnings.map((warning, index) => (
            <li key={index}>{warning.message}</li>
          ))}
        </ul>
        <Actions>{cancel("Done")}</Actions>
      </div>
    );

  if (outcome.kind === "failed")
    return (
      <div className="space-y-2 select-text">
        <p className={`${textSize} text-danger leading-relaxed whitespace-pre-wrap`}>
          {outcome.message}
        </p>
        <div className="flex items-center justify-between gap-3">
          {outcome.canRemoveFromApp && (
            <span className={`${textSize} text-text-secondary`}>Remove from app anyway?</span>
          )}
          <Actions>
            {cancel()}
            {outcome.canRemoveFromApp && (
              <button
                onClick={() => onRetry({ deleteWorktrees: false })}
                disabled={deleting}
                className={dangerButton}
              >
                Remove
              </button>
            )}
          </Actions>
        </div>
      </div>
    );

  return (
    <div className="space-y-2 select-text">
      <p
        className={`${textSize} leading-relaxed ${outcome.teardown === "failed" ? "text-danger" : "text-text-secondary"}`}
      >
        {outcome.message}
      </p>
      {outcome.scripts.map((script) => (
        <div key={script.repoName} className="space-y-1">
          <p className={`${detailSize} text-text-muted`}>
            {script.repoName} · {SCRIPT_SOURCE_LABELS[script.source]}
          </p>
          <pre
            className={`max-h-32 overflow-auto rounded-md border border-border bg-bg-tertiary p-2 ${detailSize} font-mono whitespace-pre-wrap`}
          >
            {script.script}
          </pre>
        </div>
      ))}
      <Actions>
        {cancel()}
        <button
          onClick={() => onRetry({ deleteWorktrees: true, teardown: { skipTeardown: true } })}
          disabled={deleting}
          className={dangerGhostButton}
        >
          Delete without teardown
        </button>
        {outcome.teardown === "approval" && (
          <button
            onClick={() => onRetry({ deleteWorktrees: true, teardown: { approveTeardown: true } })}
            disabled={deleting}
            className={dangerButton}
          >
            Run and delete
          </button>
        )}
      </Actions>
    </div>
  );
}

function Actions({ children }: { children: ReactNode }) {
  return <div className="flex justify-end gap-2 flex-shrink-0 ml-auto">{children}</div>;
}
