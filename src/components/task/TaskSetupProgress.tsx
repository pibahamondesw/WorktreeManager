import { useSyncExternalStore } from "react";
import {
  dismissTaskSetup,
  getTaskSetup,
  SETUP_STAGES,
  subscribeTaskSetup,
} from "../../services/taskSetup";

import { CloseIcon } from "../ui/Icons";

export function TaskSetupProgress({ taskId }: { taskId: string }) {
  const setup = useSyncExternalStore(subscribeTaskSetup, () => getTaskSetup(taskId));
  if (!setup || setup.dismissed) return null;
  const hasErrors = setup.repos.some((repo) => repo.steps.some((step) => step.status === "error"));
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
      className="flex items-start gap-2 px-4 py-2 text-xs text-text-secondary select-text"
      onClick={(event) => event.stopPropagation()}
      onKeyDown={(event) => event.stopPropagation()}
    >
      <details className="min-w-0 flex-1">
        <summary className="cursor-pointer">
          <span role="status" className={hasErrors ? "text-danger" : undefined}>
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
