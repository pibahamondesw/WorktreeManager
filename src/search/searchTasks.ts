import { compareTaskPins } from "../utils";
import { Task, Workspace } from "../types";
import { TerminalStatus } from "../services/terminal";
import {
  AgentActivities,
  INDICATOR_RANK,
  TaskIndicator,
  taskIndicator,
} from "../services/agentActivity";
import { Filter, ParsedQuery, parseQuery } from "./query";

export interface TaskSearchResult {
  task: Task;
  workspace: Workspace | undefined;
  score: number;
  inCurrentWorkspace: boolean;
  activeSession: boolean;
  indicator: TaskIndicator | null;
}

interface SearchArgs {
  tasks: Task[];
  workspaces: Workspace[];
  selectedWorkspaceId: string | null;
  query: string;
  lastVisitAt?: Map<string, string>;
  agentSessions?: Record<string, TerminalStatus>;
  agentActivities?: AgentActivities;
}

/** Ranked so an exact Linear identifier always beats an incidental title hit. */
const SCORES = {
  identifierExact: 100,
  identifierPrefix: 70,
  titleWordStart: 50,
  branchPrefix: 45,
  titleSubstring: 30,
  branchSubstring: 25,
  repoSubstring: 15,
  workspaceSubstring: 12,
  pathSubstring: 8,
};

/** Outranks any achievable text score, so the current workspace always sorts first. */
const CURRENT_WORKSPACE_BOOST = 1_000_000;
/** Per indicator rank: needs input, then idle sessions waiting on you, then working agents. */
const AGENT_BOOST = 2_000_000;

interface TaskFields {
  identifier: string;
  title: string;
  branches: string[];
  repos: string[];
  paths: string[];
  workspace: string;
  project: string;
  activeSession: boolean;
}

function fieldsOf(
  task: Task,
  workspace: Workspace | undefined,
  activeSession: boolean
): TaskFields {
  return {
    identifier: (task.linearIssueIdentifier ?? "").toLowerCase(),
    title: (task.linearIssueTitle ?? "").toLowerCase(),
    branches: [task.branchName, ...task.members.map((m) => m.branchName)].map((b) =>
      b.toLowerCase()
    ),
    repos: task.members.map((m) => m.repoName.toLowerCase()),
    paths: task.members.map((m) => m.path.toLowerCase()),
    project: (task.linearProjectName ?? "").toLowerCase(),
    workspace: (workspace?.name ?? "").toLowerCase(),
    activeSession,
  };
}

function startsWordWith(haystack: string, term: string): boolean {
  if (haystack.startsWith(term)) return true;
  const boundary = /[\s\-_/:.]/;
  for (let i = 1; i < haystack.length; i++) {
    if (boundary.test(haystack[i - 1]) && haystack.startsWith(term, i)) return true;
  }
  return false;
}

/** Best score for one free-text term across a task's fields; 0 means no match. */
function scoreTerm(f: TaskFields, term: string): number {
  if (f.identifier === term) return SCORES.identifierExact;
  if (f.identifier && f.identifier.startsWith(term)) return SCORES.identifierPrefix;
  if (f.title && startsWordWith(f.title, term)) return SCORES.titleWordStart;
  if (f.branches.some((b) => startsWordWith(b, term))) return SCORES.branchPrefix;
  if (f.title.includes(term)) return SCORES.titleSubstring;
  if (f.branches.some((b) => b.includes(term))) return SCORES.branchSubstring;
  if (f.repos.some((r) => r.includes(term))) return SCORES.repoSubstring;
  if (f.project.includes(term)) return SCORES.workspaceSubstring;
  if (f.workspace.includes(term)) return SCORES.workspaceSubstring;
  if (f.paths.some((p) => p.includes(term))) return SCORES.pathSubstring;
  return 0;
}

function matchesFilter(f: TaskFields, filter: Filter): boolean {
  return filter.values.some((value) => {
    switch (filter.field) {
      case "project":
        return value === "none" ? !f.project : f.project.includes(value);
      case "in":
        return f.workspace.includes(value);
      case "repo":
        return f.repos.some((r) => r.includes(value));
      case "branch":
        return f.branches.some((b) => b.includes(value));
      case "id":
        return f.identifier.includes(value);
      case "path":
        return f.paths.some((p) => p.includes(value));
      case "session":
        return value === "active" && f.activeSession;
    }
  });
}

function scoreTask(f: TaskFields, parsed: ParsedQuery): number | null {
  if (!parsed.filters.every((filter) => matchesFilter(f, filter))) return null;
  if (parsed.negFilters.some((filter) => matchesFilter(f, filter))) return null;
  if (parsed.negTerms.some((term) => scoreTerm(f, term) > 0)) return null;

  let score = 0;
  for (const term of parsed.terms) {
    const termScore = scoreTerm(f, term);
    if (termScore === 0) return null;
    score += termScore;
  }
  return score;
}

/**
 * Rank every task against the palette query. Agents needing input come first, then idle live
 * sessions, then working agents, followed by tasks in the current workspace, then by pinned order, text score
 * and most recent visit.
 */
export function searchTasks({
  tasks,
  workspaces,
  selectedWorkspaceId,
  query,
  lastVisitAt,
  agentSessions = {},
  agentActivities = {},
}: SearchArgs): TaskSearchResult[] {
  const recencyOf = (task: Task) => lastVisitAt?.get(task.id) ?? task.createdAt;
  const parsed = parseQuery(query);
  const workspaceById = new Map(workspaces.map((w) => [w.id, w]));
  const results: TaskSearchResult[] = [];

  for (const task of tasks) {
    const workspace = workspaceById.get(task.workspaceId);
    const activeSession = agentSessions[task.id]?.kind === "running";
    const score = scoreTask(fieldsOf(task, workspace, activeSession), parsed);
    if (score === null) continue;
    results.push({
      task,
      workspace,
      score,
      inCurrentWorkspace: task.workspaceId === selectedWorkspaceId,
      activeSession,
      indicator: taskIndicator(agentActivities[task.id], agentSessions[task.id]),
    });
  }

  const rank = (result: TaskSearchResult) =>
    (result.inCurrentWorkspace ? CURRENT_WORKSPACE_BOOST : 0) +
    (result.indicator ? INDICATOR_RANK[result.indicator] * AGENT_BOOST : 0);
  return results.sort((a, b) => {
    const rankA = rank(a);
    const rankB = rank(b);
    if (rankA !== rankB) return rankB - rankA;
    const pins = compareTaskPins(a.task, b.task);
    if (pins !== 0) return pins;
    if (a.score !== b.score) return b.score - a.score;
    return recencyOf(b.task).localeCompare(recencyOf(a.task));
  });
}
