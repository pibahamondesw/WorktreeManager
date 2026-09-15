import { PaletteCommand } from "./commands";
import { parsePaletteInput, parseQuery } from "./query";
import { searchTasks, TaskSearchResult } from "./searchTasks";
import { Task, Workspace } from "../types";

export type PaletteItem =
  | { kind: "task"; id: string; result: TaskSearchResult }
  | { kind: "command"; id: string; command: PaletteCommand; score: number };

interface SearchPaletteArgs {
  tasks: Task[];
  workspaces: Workspace[];
  selectedWorkspaceId: string | null;
  query: string;
  lastVisitAt?: Map<string, string>;
  commands: PaletteCommand[];
}

const GROUP_ORDER: Record<PaletteCommand["group"], number> = {
  action: 0,
  workspace: 1,
  settings: 2,
};

function startsWordWith(haystack: string, term: string): boolean {
  if (haystack.startsWith(term)) return true;
  const boundary = /[\s\-_/:.]/;
  for (let i = 1; i < haystack.length; i++) {
    if (boundary.test(haystack[i - 1]) && haystack.startsWith(term, i)) return true;
  }
  return false;
}

/** Best score for one term against a command's keywords; 0 means no match. */
function scoreTerm(keywords: string, term: string): number {
  if (keywords === term) return 100;
  if (startsWordWith(keywords, term)) return 50;
  if (keywords.includes(term)) return 20;
  return 0;
}

function scoreCommand(command: PaletteCommand, terms: string[], negTerms: string[]): number | null {
  if (negTerms.some((term) => scoreTerm(command.keywords, term) > 0)) return null;
  if (terms.length === 0) return 0;
  let score = 0;
  for (const term of terms) {
    const termScore = scoreTerm(command.keywords, term);
    if (termScore === 0) return null;
    score += termScore;
  }
  return score;
}

function sortCommands(items: Extract<PaletteItem, { kind: "command" }>[]): typeof items {
  return items.sort((a, b) => {
    const group = GROUP_ORDER[a.command.group] - GROUP_ORDER[b.command.group];
    if (group !== 0) return group;
    if (a.score !== b.score) return b.score - a.score;
    return a.command.label.localeCompare(b.command.label);
  });
}

/**
 * Tasks first (so ↵ still opens the best task match), then matching commands.
 * A leading `>` hides tasks and lists every matching command.
 */
export function searchPalette({
  tasks,
  workspaces,
  selectedWorkspaceId,
  query: rawQuery,
  lastVisitAt,
  commands,
}: SearchPaletteArgs): PaletteItem[] {
  const { commandsOnly, query } = parsePaletteInput(rawQuery);
  const parsed = parseQuery(query);
  const hasTerms = parsed.terms.length > 0;

  const commandItems: Extract<PaletteItem, { kind: "command" }>[] = [];
  for (const command of commands) {
    const score = scoreCommand(command, parsed.terms, parsed.negTerms);
    if (score === null) continue;
    if (!commandsOnly && !hasTerms && !command.emptyVisible) continue;
    commandItems.push({ kind: "command", id: command.id, command, score });
  }
  sortCommands(commandItems);

  if (commandsOnly) return commandItems;

  const taskItems: PaletteItem[] = searchTasks({
    tasks,
    workspaces,
    selectedWorkspaceId,
    query,
    lastVisitAt,
  }).map((result) => ({ kind: "task" as const, id: result.task.id, result }));

  return [...taskItems, ...commandItems];
}
