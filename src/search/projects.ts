import { Task } from "../types";

export const NO_PROJECT = "__no_project__";

export function taskProjectKey(task: Task): string {
  return task.linearProjectId ?? task.linearProjectName ?? NO_PROJECT;
}

export function matchesProject(task: Task, project: string): boolean {
  return !project || taskProjectKey(task) === project;
}
