import { Task } from "../types";
import type { OperationWarning } from "./operations";

export const SETUP_STAGES = {
  config: "Copying local configuration",
  doppler_setup: "Configuring Doppler",
  install_node_deps: "Installing Node dependencies",
  install_python_deps: "Installing Python dependencies",
} as const;

export type SetupStage = keyof typeof SETUP_STAGES;
export type SetupStatus = "pending" | "running" | "completed" | "skipped" | "error";
export interface TaskSetupState {
  active: boolean;
  opened: boolean;
  dismissed: boolean;
  repos: {
    repoId: string;
    repoName: string;
    steps: { stage: SetupStage; status: SetupStatus; message?: string }[];
  }[];
  warnings: OperationWarning[];
}

const states = new Map<string, TaskSetupState>();
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

export function initializeTaskSetup(task: Task, warnings: OperationWarning[]) {
  updateTaskSetup(task.id, {
    active: true,
    opened: false,
    dismissed: false,
    repos: task.members.map((member) => ({
      repoId: member.repoId,
      repoName: member.repoName,
      steps: (Object.keys(SETUP_STAGES) as SetupStage[]).map((stage) => ({
        stage,
        status: "pending",
      })),
    })),
    warnings: [...warnings],
  });
}

export function updateSetupStep(
  taskId: string,
  repoId: string,
  stage: SetupStage,
  status: SetupStatus,
  warnings: OperationWarning[],
  message?: string
) {
  const state = states.get(taskId)!;
  updateTaskSetup(taskId, {
    ...state,
    repos: state.repos.map((repo) =>
      repo.repoId !== repoId
        ? repo
        : {
            ...repo,
            steps: repo.steps.map((step) =>
              step.stage === stage ? { stage, status, ...(message ? { message } : {}) } : step
            ),
          }
    ),
    warnings: [...warnings],
  });
}

export function clearTaskSetup(taskId: string) {
  states.delete(taskId);
  listeners.forEach((listener) => listener());
}

export function dismissTaskSetup(taskId: string) {
  const state = states.get(taskId);
  if (state) updateTaskSetup(taskId, { ...state, dismissed: true });
}
