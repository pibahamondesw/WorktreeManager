import { useEffect, useSyncExternalStore } from "react";
import {
  dismissTaskSetup,
  getTaskSetup,
  markTaskSetupDetailsViewed,
  SETUP_STAGES,
  subscribeTaskSetup,
} from "../../services/taskSetup";

import { CheckIcon, CloseIcon } from "../ui/Icons";

const AUTO_DISMISS_MS = 5000;

export function TaskSetupProgress({ taskId }: { taskId: string }) {
  const setup = useSyncExternalStore(subscribeTaskSetup, () => getTaskSetup(taskId));
  const hasErrors = setup?.repos.some((repo) => repo.steps.some((step) => step.status === "error"));
  const completedSuccessfully = !!setup && !setup.active && !hasErrors && !setup.warnings.length;
  const autoDismiss = completedSuccessfully && !setup.dismissed && !setup.detailsViewed;

  useEffect(() => {
    if (!autoDismiss) return;
    const timeout = window.setTimeout(() => dismissTaskSetup(taskId), AUTO_DISMISS_MS);
    return () => window.clearTimeout(timeout);
  }, [autoDismiss, taskId]);

  if (!setup || setup.dismissed) return null;
  const summary = hasErrors
    ? setup.active
      ? "Setup running · Errors detected"
      : "Setup completed with errors"
    : setup.active
      ? setup.opened
        ? "Workspace opened · Setup running"
        : "Setup running"
      : setup.warnings.length
        ? "Setup completed with warnings"
        : "Setup completed";

  return (
    <div
      className="relative flex items-start gap-2 px-4 py-2 text-xs text-text-secondary select-text"
      onClick={(event) => event.stopPropagation()}
      onKeyDown={(event) => event.stopPropagation()}
    >
      <details className="min-w-0 flex-1">
        <summary
          className="cursor-pointer"
          onClick={(event) => {
            if (!(event.currentTarget.parentElement as HTMLDetailsElement).open) {
              markTaskSetupDetailsViewed(taskId);
            }
          }}
        >
          <span role="status" className={hasErrors ? "text-danger" : undefined}>
            {completedSuccessfully && (
              <CheckIcon size={14} className="inline-block mr-1 align-text-bottom text-success" />
            )}
            {summary}
          </span>
        </summary>
        <ul className="mt-2 space-y-1">
          {setup.repos.flatMap((repo) =>
            repo.steps
              .filter((step) => step.status !== "skipped" && step.status !== "pending")
              .map((step) => (
                <li
                  key={`${repo.repoId}:${step.stage}`}
                  className={step.status === "error" ? "text-danger" : undefined}
                >
                  {repo.repoName} · {SETUP_STAGES[step.stage]} · {step.status}
                  {step.message && (
                    <p className="whitespace-pre-wrap break-words">{step.message}</p>
                  )}
                </li>
              ))
          )}
          {setup.warnings.map((warning, index) => (
            <li key={index} className="text-danger">
              {warning.repoId &&
                `${setup.repos.find((repo) => repo.repoId === warning.repoId)?.repoName ?? warning.repoId} · `}
              {warning.message}
            </li>
          ))}
        </ul>
      </details>
      {autoDismiss && (
        <div
          key={taskId}
          role="progressbar"
          aria-label="Setup notification auto-dismiss countdown"
          className="absolute bottom-0 left-0 h-0.5 w-full origin-left bg-accent"
          style={{ animation: `setup-countdown ${AUTO_DISMISS_MS}ms linear forwards` }}
        />
      )}
      <button
        type="button"
        aria-label="Dismiss setup progress"
        title="Dismiss setup progress"
        className="shrink-0 rounded text-text-muted hover:text-text-primary cursor-pointer"
        onClick={() => dismissTaskSetup(taskId)}
      >
        <CloseIcon size={14} />
      </button>
    </div>
  );
}
