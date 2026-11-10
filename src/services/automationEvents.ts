import { invoke } from "@tauri-apps/api/core";
import { GithubPrStatus, PullRequestInfo, Task } from "../types";
import { githubPrStatus, prKey } from "./github";

type PrAction =
  | "merged"
  | "closed"
  | "reopened"
  | "ready"
  | "checks_passed"
  | "checks_failed"
  | "approved"
  | "changes_requested";

type AppEventType =
  | "task.created"
  | "task.deleted"
  | "agent.done"
  | "agent.waiting"
  | `pr.${PrAction}`;

interface EventTask {
  id: string;
  workspaceId: string;
  branch: string;
  automationId?: string;
  linearIssue?: string;
}

export interface AppEvent {
  type: AppEventType;
  task: EventTask;
  agent?: { id: string; surface: string; at: number };
  pr?: GithubPrStatus & { repo: string; number: number; url: string; headSha: string };
}

export interface TaskPrs {
  task: Task;
  prs: PullRequestInfo[];
}

/** Hands an app event to the automation engine; automations never fail the action behind it. */
export function emitAppEvent(event: AppEvent): void {
  void Promise.resolve()
    .then(() => invoke("automation_app_event", { event }))
    .catch(() => undefined);
}

function eventTask(task: Task): EventTask {
  return {
    id: task.id,
    workspaceId: task.workspaceId,
    branch: task.branchName,
    ...(task.automationId ? { automationId: task.automationId } : {}),
    ...(task.linearIssueIdentifier ? { linearIssue: task.linearIssueIdentifier } : {}),
  };
}

export const taskAppEvent = (type: "task.created" | "task.deleted", task: Task): AppEvent => ({
  type,
  task: eventTask(task),
});

export const agentAppEvent = (
  task: Task,
  event: { state: "done" | "waiting"; agent: string; surface: string; at: number }
): AppEvent => ({
  type: `agent.${event.state}`,
  task: eventTask(task),
  agent: { id: event.agent, surface: event.surface, at: event.at },
});

const PR_ACTIONS: [PrAction, (previous: GithubPrStatus, next: GithubPrStatus) => boolean][] = [
  ["merged", (previous, next) => previous.state !== "merged" && next.state === "merged"],
  ["closed", (previous, next) => previous.state !== "closed" && next.state === "closed"],
  ["reopened", (previous, next) => previous.state === "closed" && next.state === "open"],
  ["ready", (previous, next) => previous.isDraft && !next.isDraft && next.state === "open"],
  [
    "checks_passed",
    (previous, next) =>
      next.ci === "passing" && (previous.ci !== "passing" || previous.headSha !== next.headSha),
  ],
  [
    "checks_failed",
    (previous, next) =>
      next.ci === "failing" && (previous.ci !== "failing" || previous.headSha !== next.headSha),
  ],
  ["approved", (previous, next) => previous.review !== "approved" && next.review === "approved"],
  [
    "changes_requested",
    (previous, next) =>
      previous.review !== "changes_requested" && next.review === "changes_requested",
  ],
];

export function prActions(previous: GithubPrStatus, next: GithubPrStatus): PrAction[] {
  return PR_ACTIONS.filter(([, changed]) => changed(previous, next)).map(([action]) => action);
}

/**
 * Reports what changed in each task's pull requests since it last saw them. The first status
 * seen is only a baseline, so opening the app does not replay every pull request.
 */
export class PrEventTracker {
  private seen = new Map<string, GithubPrStatus>();
  private emit: (event: AppEvent) => void;

  constructor(emit: (event: AppEvent) => void = emitAppEvent) {
    this.emit = emit;
  }

  observe(task: Task, pr: PullRequestInfo, status: GithubPrStatus | undefined) {
    if (!status?.headSha) return;
    const key = `${task.id}:${prKey(pr)}`;
    const previous = this.seen.get(key);
    this.seen.set(key, status);
    if (!previous) return;
    for (const action of prActions(previous, status))
      this.emit({
        type: `pr.${action}`,
        task: eventTask(task),
        pr: {
          ...status,
          headSha: status.headSha,
          repo: pr.repoSlug,
          number: pr.number,
          url: pr.url,
        },
      });
  }
}

const prEventTracker = new PrEventTracker();

export function reportPrChanges(taskPrs: TaskPrs[]) {
  for (const { task, prs } of taskPrs)
    for (const pr of prs) prEventTracker.observe(task, pr, githubPrStatus(pr));
}
