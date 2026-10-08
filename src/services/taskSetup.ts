import { Task } from "../types";
import type { OperationWarning } from "./operations";
import type { ScriptSource } from "./repoScripts";

export const SETUP_STAGES = {
  config: "Copying local configuration",
  doppler_setup: "Configuring Doppler",
  install_node_deps: "Installing Node dependencies",
  install_python_deps: "Installing Python dependencies",
  script: "Running setup script",
  teardown: "Running teardown script",
} as const;

export type SetupStage = keyof typeof SETUP_STAGES;
export const DETECTED_STAGES: SetupStage[] = [
  "doppler_setup",
  "install_node_deps",
  "install_python_deps",
];
export type SetupStatus = "pending" | "running" | "completed" | "skipped" | "error" | "cancelled";

export interface RepoSetupState {
  repoId: string;
  repoName: string;
  /** Where the setup comes from; absent while resolving and for detected setup. */
  source?: ScriptSource;
  steps: SetupStep[];
}

export interface SetupStep {
  stage: SetupStage;
  status: SetupStatus;
  message?: string;
  /** Counts script runs, so an output view can reattach to the newest session. */
  run?: number;
}

export interface TaskSetupState {
  active: boolean;
  opened: boolean;
  dismissed: boolean;
  detailsViewed: boolean;
  repos: RepoSetupState[];
  warnings: OperationWarning[];
}

const states = new Map<string, TaskSetupState>();
const runs = new Map<string, number>();
const listeners = new Set<() => void>();

export const subscribeTaskSetup = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
export const getTaskSetup = (taskId: string) => states.get(taskId);

export function updateTaskSetup(taskId: string, state: TaskSetupState) {
  states.set(taskId, state);
  listeners.forEach((listener) => listener());
}

function pendingSteps(stages: SetupStage[]) {
  return stages.map((stage) => ({ stage, status: "pending" as SetupStatus }));
}

export function initializeTaskSetup(task: Task, warnings: OperationWarning[]) {
  runs.set(task.id, 1);
  updateTaskSetup(task.id, {
    active: true,
    opened: false,
    dismissed: false,
    detailsViewed: false,
    repos: task.members.map((member) => ({
      repoId: member.repoId,
      repoName: member.repoName,
      steps: pendingSteps(["config", ...DETECTED_STAGES]),
    })),
    warnings: [...warnings],
  });
}

/** Mark the task busy for a retry or teardown; pairs with `endSetupRun`. */
export function beginSetupRun(task: Task) {
  runs.set(task.id, (runs.get(task.id) ?? 0) + 1);
  const state = states.get(task.id);
  updateTaskSetup(task.id, {
    ...(state ?? {
      opened: true,
      dismissed: false,
      detailsViewed: false,
      repos: task.members.map((member) => ({
        repoId: member.repoId,
        repoName: member.repoName,
        steps: [],
      })),
      warnings: [],
    }),
    active: true,
    dismissed: false,
  });
}

export function endSetupRun(taskId: string, warnings?: OperationWarning[]) {
  const remaining = Math.max((runs.get(taskId) ?? 1) - 1, 0);
  runs.set(taskId, remaining);
  const state = states.get(taskId);
  if (!state) return;
  updateTaskSetup(taskId, {
    ...state,
    active: remaining > 0,
    ...(warnings ? { warnings: [...warnings] } : {}),
  });
}

export function updateRepoSetup(
  taskId: string,
  repoId: string,
  update: (repo: RepoSetupState) => RepoSetupState
) {
  const state = states.get(taskId);
  if (!state) return;
  updateTaskSetup(taskId, {
    ...state,
    repos: state.repos.map((repo) => (repo.repoId === repoId ? update(repo) : repo)),
  });
}

/** Replace a repository's remaining steps once its setup source is known, keeping `config`. */
export function planRepoSetup(
  taskId: string,
  repoId: string,
  stages: SetupStage[],
  source?: ScriptSource
) {
  updateRepoSetup(taskId, repoId, (repo) => ({
    ...repo,
    source,
    steps: [
      ...repo.steps.filter((step) => step.stage === "config"),
      ...pendingSteps(stages).map((step) => {
        const run = repo.steps.find((previous) => previous.stage === step.stage)?.run;
        return run ? { ...step, run } : step;
      }),
    ],
  }));
}

export function updateSetupStep(
  taskId: string,
  repoId: string,
  stage: SetupStage,
  status: SetupStatus,
  warnings: OperationWarning[],
  message?: string
) {
  const state = states.get(taskId);
  if (!state) return;
  const stepFor = (previous?: SetupStep): SetupStep => {
    const run = status === "running" ? (previous?.run ?? 0) + 1 : previous?.run;
    return { stage, status, ...(message ? { message } : {}), ...(run ? { run } : {}) };
  };
  updateTaskSetup(taskId, {
    ...state,
    repos: state.repos.map((repo) =>
      repo.repoId !== repoId
        ? repo
        : {
            ...repo,
            steps: repo.steps.some((item) => item.stage === stage)
              ? repo.steps.map((item) => (item.stage === stage ? stepFor(item) : item))
              : [...repo.steps, stepFor()],
          }
    ),
    warnings: [...warnings],
  });
}

export function clearTaskSetup(taskId: string) {
  states.delete(taskId);
  runs.delete(taskId);
  listeners.forEach((listener) => listener());
}

export function dismissTaskSetup(taskId: string) {
  const state = states.get(taskId);
  if (state) updateTaskSetup(taskId, { ...state, dismissed: true });
}

export function markTaskSetupDetailsViewed(taskId: string) {
  const state = states.get(taskId);
  if (state && !state.detailsViewed) updateTaskSetup(taskId, { ...state, detailsViewed: true });
}
