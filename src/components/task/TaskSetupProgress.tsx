import { useEffect, useState, useSyncExternalStore } from "react";
import {
  dismissTaskSetup,
  getTaskSetup,
  markTaskSetupDetailsViewed,
  RepoSetupState,
  SETUP_STAGES,
  SetupStep,
  subscribeTaskSetup,
} from "../../services/taskSetup";
import { SCRIPT_SOURCE_LABELS, scriptSession } from "../../services/repoScripts";
import type { SetupRetry } from "../../services/operations";
import { CheckIcon, CloseIcon } from "../ui/Icons";
import { ScriptOutput } from "./ScriptOutput";

const AUTO_DISMISS_MS = 5000;

export interface SetupControls {
  rerun: (taskId: string, repoId: string, choice: SetupRetry) => Promise<unknown>;
  cancel: (taskId: string, repoId: string) => Promise<unknown>;
}

const isScriptStage = (step: SetupStep) => step.stage === "script" || step.stage === "teardown";

export function TaskSetupProgress({
  taskId,
  controls,
}: {
  taskId: string;
  controls?: SetupControls;
}) {
  const setup = useSyncExternalStore(subscribeTaskSetup, () => getTaskSetup(taskId));
  const steps = setup?.repos.flatMap((repo) => repo.steps) ?? [];
  const hasErrors = steps.some((step) => step.status === "error");
  const awaitingApproval = !!setup?.repos.some((repo) => repo.approval);
  const ranScript = steps.some((step) => isScriptStage(step) && step.status !== "pending");
  const completedSuccessfully = !!setup && !setup.active && !hasErrors && !setup.warnings.length;
  const autoDismiss =
    completedSuccessfully && !ranScript && !setup.dismissed && !setup.detailsViewed;

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
      : awaitingApproval
        ? "Setup waiting for approval"
        : setup.warnings.length
          ? "Setup completed with warnings"
          : "Setup completed";

  return (
    <div
      className="relative flex items-start gap-2 px-4 py-2 text-xs text-text-secondary select-text"
      onClick={(event) => event.stopPropagation()}
      onKeyDown={(event) => event.stopPropagation()}
    >
      <details className="min-w-0 flex-1" open={awaitingApproval || undefined}>
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
              .map((step) =>
                isScriptStage(step) ? (
                  <ScriptStep
                    key={`${repo.repoId}:${step.stage}`}
                    taskId={taskId}
                    repo={repo}
                    step={step}
                    controls={controls}
                  />
                ) : (
                  <li
                    key={`${repo.repoId}:${step.stage}`}
                    className={step.status === "error" ? "text-danger" : undefined}
                  >
                    {repo.repoName} · {SETUP_STAGES[step.stage]} · {step.status}
                    {step.message && (
                      <p className="whitespace-pre-wrap break-words">{step.message}</p>
                    )}
                  </li>
                )
              )
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

function StepAction({ onClick, children }: { onClick: () => void; children: string }) {
  return (
    <button
      type="button"
      className="ml-2 rounded px-1.5 text-accent hover:bg-bg-hover cursor-pointer"
      onClick={onClick}
    >
      {children}
    </button>
  );
}

function ScriptStep({
  taskId,
  repo,
  step,
  controls,
}: {
  taskId: string;
  repo: RepoSetupState;
  step: SetupStep;
  controls?: SetupControls;
}) {
  const [showOutput, setShowOutput] = useState(step.status === "running");
  const [actionError, setActionError] = useState<string | null>(null);
  const running = step.status === "running";
  const isSetup = step.stage === "script";
  const session = scriptSession(isSetup ? "setup" : "teardown", repo.repoId);
  const act = (action: () => Promise<unknown>) => {
    setActionError(null);
    action().catch((error) =>
      setActionError(error instanceof Error ? error.message : String(error))
    );
  };
  const rerun = (choice: SetupRetry) =>
    controls &&
    act(() => {
      if (choice !== "detected") setShowOutput(true);
      return controls.rerun(taskId, repo.repoId, choice);
    });

  return (
    <li className={step.status === "error" ? "text-danger" : undefined}>
      {repo.repoName} · {SETUP_STAGES[step.stage]}
      {repo.source && ` from ${SCRIPT_SOURCE_LABELS[repo.source]}`} · {step.status}
      {step.run !== undefined && (
        <StepAction onClick={() => setShowOutput((shown) => !shown)}>
          {showOutput ? "Hide output" : "Output"}
        </StepAction>
      )}
      {controls && running && isSetup && (
        <StepAction onClick={() => act(() => controls.cancel(taskId, repo.repoId))}>
          Stop
        </StepAction>
      )}
      {controls && isSetup && !running && step.status !== "needs_approval" && (
        <StepAction onClick={() => rerun("retry")}>Run again</StepAction>
      )}
      {step.message && <p className="whitespace-pre-wrap break-words">{step.message}</p>}
      {repo.approval && isSetup && (
        <div className="mt-1 space-y-1 text-text-secondary">
          <p>{SCRIPT_SOURCE_LABELS[repo.approval.source]} wants to run this in the new worktree:</p>
          <pre className="max-h-40 overflow-auto rounded-md border border-border bg-bg-tertiary p-2 font-mono whitespace-pre-wrap">
            {repo.approval.script}
          </pre>
          {controls && (
            <p>
              <StepAction onClick={() => rerun("approve")}>Approve and run</StepAction>
              <StepAction onClick={() => rerun("detected")}>Use detected setup</StepAction>
            </p>
          )}
        </div>
      )}
      {actionError && <p className="text-danger">{actionError}</p>}
      {showOutput && step.run !== undefined && (
        <div className="mt-1">
          <ScriptOutput taskId={taskId} session={session} running={running} generation={step.run} />
        </div>
      )}
    </li>
  );
}
