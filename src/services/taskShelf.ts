import { GitStatus, IssueLinearInfo, Task } from "../types";
import { isTaskSnoozed } from "../utils";
import { groupTaskPrs } from "./taskLinear";
import { githubPrStatus } from "./github";

const DAY_MS = 24 * 60 * 60 * 1000;

export interface AutoShelveSettings {
  /** `null` turns the inactivity rule off. */
  inactiveDays: number | null;
  /** When the inactivity rule started counting; older activity does not move tasks. */
  since: string | null;
}

export type AutoShelveReason = "linear" | "pr" | "inactive";

/** What is known about a task; `undefined` means not loaded yet, so that rule waits. */
export interface AutoShelveSignals {
  linearStateType?: string;
  prStates?: string[];
  lastCommitAt?: number;
}

export function canAutoShelve(task: Task, now: number): boolean {
  return (
    task.shelvedAt === undefined &&
    task.pinOrder === undefined &&
    task.autoShelve !== false &&
    !isTaskSnoozed(task, now)
  );
}

export function autoShelveReason(
  task: Task,
  signals: AutoShelveSignals,
  now: number,
  settings: AutoShelveSettings
): AutoShelveReason | null {
  if (!canAutoShelve(task, now)) return null;
  if (signals.linearStateType === "completed" || signals.linearStateType === "canceled")
    return "linear";
  if (
    signals.prStates?.length &&
    signals.prStates.every((state) => state === "merged" || state === "closed")
  )
    return "pr";
  return isInactive(task, signals, now, settings) ? "inactive" : null;
}

function isInactive(
  task: Task,
  { lastCommitAt }: AutoShelveSignals,
  now: number,
  { inactiveDays, since }: AutoShelveSettings
) {
  if (inactiveDays === null || since === null || lastCommitAt === undefined) return false;
  const lastActivity = Math.max(
    Date.parse(task.createdAt) || 0,
    Date.parse(since),
    lastCommitAt,
    Date.parse(task.lastActiveAt ?? "") || 0
  );
  return now - lastActivity >= inactiveDays * DAY_MS;
}

export interface TaskSignalSources {
  linearInfo: Record<string, IssueLinearInfo>;
  gitStatuses: Record<string, GitStatus>;
  repoSlugs: (task: Task) => Record<string, string> | null;
}

export function autoShelveSignals(task: Task, sources: TaskSignalSources): AutoShelveSignals {
  const info = task.linearIssueId ? sources.linearInfo[task.linearIssueId] : undefined;
  return {
    linearStateType: info?.status?.type,
    prStates: info ? prStates(task, info, sources.repoSlugs(task)) : undefined,
    lastCommitAt: lastCommitAt(task, sources.gitStatuses),
  };
}

function prStates(
  task: Task,
  info: IssueLinearInfo,
  repoSlugs: Record<string, string> | null
): string[] | undefined {
  if (repoSlugs === null) return undefined;
  const byMember = Object.values(groupTaskPrs(task, info.prs, repoSlugs).byMember);
  if (byMember.some((prs) => prs.length === 0)) return undefined;
  const states = byMember.flat().map((pr) => githubPrStatus(pr)?.state);
  return states.every((state) => state !== undefined) ? states : undefined;
}

function lastCommitAt(task: Task, gitStatuses: Record<string, GitStatus>): number | undefined {
  const statuses = task.members.map((member) => gitStatuses[member.path]);
  if (statuses.length === 0 || statuses.some((status) => !status)) return undefined;
  return Math.max(...statuses.map((status) => status.last_commit_epoch * 1000));
}
