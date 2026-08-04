import { Task, Workspace } from "../types";
import { Filter, ParsedQuery, parseQuery } from "./query";

export interface TaskSearchResult {
  task: Task;
  workspace: Workspace | undefined;
  score: number;
  inCurrentWorkspace: boolean;
}

interface SearchArgs {
  tasks: Task[];
  workspaces: Workspace[];
  selectedWorkspaceId: string | null;
  query: string;
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

interface TaskFields {
  identifier: string;
  title: string;
  branches: string[];
  repos: string[];
  paths: string[];
  workspace: string;
}

function fieldsOf(task: Task, workspace: Workspace | undefined): TaskFields {
  return {
    identifier: (task.linearIssueIdentifier ?? "").toLowerCase(),
    title: (task.linearIssueTitle ?? "").toLowerCase(),
    branches: [task.branchName, ...task.members.map((m) => m.branchName)].map((b) =>
      b.toLowerCase()
    ),
    repos: task.members.map((m) => m.repoName.toLowerCase()),
    paths: task.members.map((m) => m.path.toLowerCase()),
    workspace: (workspace?.name ?? "").toLowerCase(),
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
  if (f.workspace.includes(term)) return SCORES.workspaceSubstring;
  if (f.paths.some((p) => p.includes(term))) return SCORES.pathSubstring;
  return 0;
}

function matchesFilter(f: TaskFields, filter: Filter): boolean {
  return filter.values.some((value) => {
    switch (filter.field) {
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
 * Rank every task against the palette query. Tasks in the current workspace always come
 * first (the ⌘K requirement), then by text score, then most recently created.
 */
export function searchTasks({
  tasks,
  workspaces,
  selectedWorkspaceId,
  query,
}: SearchArgs): TaskSearchResult[] {
  const parsed = parseQuery(query);
  const workspaceById = new Map(workspaces.map((w) => [w.id, w]));
  const results: TaskSearchResult[] = [];

  for (const task of tasks) {
    const workspace = workspaceById.get(task.workspaceId);
    const score = scoreTask(fieldsOf(task, workspace), parsed);
    if (score === null) continue;
    results.push({
      task,
      workspace,
      score,
      inCurrentWorkspace: task.workspaceId === selectedWorkspaceId,
    });
  }

  return results.sort((a, b) => {
    const rankA = a.score + (a.inCurrentWorkspace ? CURRENT_WORKSPACE_BOOST : 0);
    const rankB = b.score + (b.inCurrentWorkspace ? CURRENT_WORKSPACE_BOOST : 0);
    if (rankA !== rankB) return rankB - rankA;
    return b.task.createdAt.localeCompare(a.task.createdAt);
  });
}
