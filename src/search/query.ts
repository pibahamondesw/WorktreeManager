export type FilterField = "in" | "repo" | "branch" | "id" | "path";

const FILTER_FIELDS: FilterField[] = ["in", "repo", "branch", "id", "path"];

/** One `key:a|b` filter. Values are OR'd against each other. */
export interface Filter {
  field: FilterField;
  values: string[];
}

export interface ParsedQuery {
  /** Free-text terms, AND'd. */
  terms: string[];
  /** Free-text terms to exclude. */
  negTerms: string[];
  filters: Filter[];
  negFilters: Filter[];
}

/** Split on whitespace, keeping `"quoted phrases"` together. */
function tokenize(input: string): string[] {
  const tokens: string[] = [];
  let current = "";
  let quoted = false;

  for (const char of input) {
    if (char === '"') {
      quoted = !quoted;
      continue;
    }
    if (!quoted && /\s/.test(char)) {
      if (current) tokens.push(current);
      current = "";
      continue;
    }
    current += char;
  }
  if (current) tokens.push(current);
  return tokens;
}

function asFilter(token: string): Filter | null {
  const colon = token.indexOf(":");
  if (colon <= 0) return null;
  const field = token.slice(0, colon).toLowerCase() as FilterField;
  if (!FILTER_FIELDS.includes(field)) return null;
  const values = token
    .slice(colon + 1)
    .split("|")
    .map((v) => v.trim().toLowerCase())
    .filter(Boolean);
  return values.length > 0 ? { field, values } : null;
}

/**
 * Parse the palette input. Unknown `key:value` tokens fall through to free text so a
 * stray colon never silently drops part of what the user typed.
 */
export function parseQuery(input: string): ParsedQuery {
  const parsed: ParsedQuery = { terms: [], negTerms: [], filters: [], negFilters: [] };

  for (const token of tokenize(input)) {
    const negated = token.startsWith("-") && token.length > 1;
    const body = negated ? token.slice(1) : token;
    const filter = asFilter(body);

    if (filter) {
      (negated ? parsed.negFilters : parsed.filters).push(filter);
    } else {
      (negated ? parsed.negTerms : parsed.terms).push(body.toLowerCase());
    }
  }

  return parsed;
}

/** The `in:` values of a query, used to keep the scope tabs in sync with the text. */
export function scopeValues(parsed: ParsedQuery): string[] {
  return parsed.filters.filter((f) => f.field === "in").flatMap((f) => f.values);
}

/** Strip every `in:` token, quoted values included. */
export function withoutScope(query: string): string {
  return query
    .replace(/(^|\s)-?in:("[^"]*"|\S*)/gi, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/** Scope a query to one workspace, replacing any `in:` already there. */
export function withScope(query: string, workspaceName: string): string {
  const token = `in:${/\s/.test(workspaceName) ? `"${workspaceName}"` : workspaceName}`;
  const rest = withoutScope(query);
  // Trailing space when there's nothing else: the caret lands ready to type.
  return rest ? `${token} ${rest}` : `${token} `;
}
