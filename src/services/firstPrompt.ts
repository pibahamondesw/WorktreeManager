/** Variables any task fills; those a task lacks, like an issue it has no link to, are empty. */
export const TASK_VARIABLES = [
  "issue.identifier",
  "issue.title",
  "issue.description",
  "issue.url",
  "issue.project",
  "task.id",
  "task.branch",
  "workspace.name",
  "repos",
  "note.path",
] as const;

/** Variables only a chat an automation opens fills, with the same values as its templates. */
export const AUTOMATION_VARIABLES = [
  "automation",
  "run.id",
  "run.short_id",
  "run.scheduled_for",
  "item.id",
  "item.branch",
  "item.version",
  "item.title",
  "tracker.issue",
  "pr.number",
  "pr.url",
] as const;

const ITEM_DATA_PREFIX = "item.data.";

type Part = { text: string } | { variable: string };

interface Parsed {
  parts: Part[];
  problem: string | null;
}

function parse(template: string): Parsed {
  const parts: Part[] = [];
  let rest = template;
  while (rest) {
    const position = rest.search(/[{}]/);
    if (position < 0) break;
    parts.push({ text: rest.slice(0, position) });
    const tail = rest.slice(position);
    if (tail.startsWith("{{") || tail.startsWith("}}")) {
      parts.push({ text: tail[0] });
      rest = tail.slice(2);
      continue;
    }
    if (tail.startsWith("}"))
      return { parts, problem: "has an unmatched }; write }} for a literal brace" };
    const end = tail.indexOf("}");
    if (end < 0) return { parts, problem: "has an unclosed {; write {{ for a literal brace" };
    parts.push({ variable: tail.slice(1, end) });
    rest = tail.slice(end + 1);
  }
  parts.push({ text: rest });
  return { parts, problem: null };
}

function isKnownVariable(name: string) {
  return (
    (TASK_VARIABLES as readonly string[]).includes(name) ||
    (AUTOMATION_VARIABLES as readonly string[]).includes(name) ||
    (name.startsWith(ITEM_DATA_PREFIX) && name.length > ITEM_DATA_PREFIX.length)
  );
}

/** Why the template cannot be used, or null when it can. */
export function firstPromptProblem(template: string): string | null {
  const { parts, problem } = parse(template);
  if (problem) return `The template ${problem}.`;
  const unknown = parts.find((part) => "variable" in part && !isKnownVariable(part.variable));
  return unknown && "variable" in unknown
    ? `The template uses the unknown variable {${unknown.variable}}.`
    : null;
}

export function firstPromptVariables(template: string): string[] {
  return parse(template).parts.flatMap((part) => ("variable" in part ? [part.variable] : []));
}

function renderLine(line: string, values: Record<string, string>): string | null {
  const parts = parse(line).parts;
  const filled = parts.map((part) => ("variable" in part ? (values[part.variable] ?? "") : null));
  const variables = filled.filter((value): value is string => value !== null);
  if (variables.length > 0 && variables.every((value) => !value.trim())) return null;
  return parts.map((part, index) => filled[index] ?? ("text" in part ? part.text : "")).join("");
}

/**
 * Fills `{variable}` placeholders; `{{` and `}}` write a literal brace. A line whose variables
 * are all empty is left out, so one template serves tasks with and without an issue.
 */
export function renderFirstPrompt(template: string, values: Record<string, string>): string {
  return template
    .split("\n")
    .flatMap((line) => renderLine(line, values) ?? [])
    .join("\n")
    .trim();
}

/** The workspace's template when it has one, otherwise the global one. */
export function effectiveFirstPromptTemplate(
  global: string | undefined,
  workspace: string | null | undefined
): string {
  return workspace?.trim() ? workspace : (global ?? "");
}

/** The first message of a new conversation: the rendered template, then what was asked. */
export function seededMessage(seed: string | null, message: string): string {
  return seed ? `${seed}\n\n${message}` : message;
}
