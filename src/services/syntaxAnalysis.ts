import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { highlightTree, tagHighlighter, tags } from "@lezer/highlight";
import { plainAnalysis, Range, SourceAnalysis, StyledRange, wordTokens } from "./semanticDiff";

type Tree = Parameters<typeof highlightTree>[0];

const MAX_PARSED_LENGTH = 1_000_000;
const MAX_TOKENIZED_LINE_LENGTH = 1000;

const highlighter = tagHighlighter([
  { tag: tags.keyword, class: "hljs-keyword" },
  {
    tag: [
      tags.function(tags.variableName),
      tags.function(tags.propertyName),
      tags.definition(tags.variableName),
    ],
    class: "hljs-title",
  },
  { tag: [tags.string, tags.special(tags.string), tags.regexp], class: "hljs-string" },
  {
    tag: [tags.number, tags.bool, tags.null, tags.atom, tags.constant(tags.variableName)],
    class: "hljs-number",
  },
  { tag: [tags.typeName, tags.className, tags.namespace], class: "hljs-type" },
  { tag: [tags.propertyName, tags.attributeName], class: "hljs-attr" },
  { tag: tags.comment, class: "hljs-comment" },
  {
    tag: [tags.punctuation, tags.operator, tags.processingInstruction, tags.labelName, tags.meta],
    class: "hljs-punctuation",
  },
]);

/** Syntax leaves become diff tokens; leaves holding whitespace (strings, comments, prose) are split into words. */
function syntaxTokens(text: string, tree: Tree): Range[] {
  const tokens: Range[] = [];
  let covered = 0;
  const words = (from: number, to: number) => {
    if (to > from) tokens.push(...wordTokens(text, from, to));
  };
  const cursor = tree.cursor();
  for (;;) {
    if (cursor.firstChild()) continue;
    if (cursor.from >= covered && cursor.to > cursor.from) {
      words(covered, cursor.from);
      if (/\s/.test(text.slice(cursor.from, cursor.to))) words(cursor.from, cursor.to);
      else tokens.push({ from: cursor.from, to: cursor.to });
      covered = cursor.to;
    }
    while (!cursor.nextSibling()) {
      if (!cursor.parent()) {
        words(covered, text.length);
        return tokens;
      }
    }
  }
}

export async function analyzeSource(path: string, text: string): Promise<SourceAnalysis> {
  const name = path.split("/").pop() ?? path;
  const description = LanguageDescription.matchFilename(languages, name);
  if (!description || text.length > MAX_PARSED_LENGTH) return plainAnalysis(text);
  try {
    const { language } = await description.load();
    const tree = language.parser.parse(text);
    const highlights: StyledRange[] = [];
    highlightTree(tree, highlighter, (from, to, className) =>
      highlights.push({ from, to, className })
    );
    return { tokens: syntaxTokens(text, tree), highlights };
  } catch {
    return plainAnalysis(text);
  }
}

function longLines(text: string): Range[] {
  const lines: Range[] = [];
  let from = 0;
  for (const line of text.split("\n")) {
    if (line.length > MAX_TOKENIZED_LINE_LENGTH) lines.push({ from, to: from + line.length });
    from += line.length + 1;
  }
  return lines;
}

function firstEndingAfter(ranges: Range[], offset: number): number {
  let low = 0;
  let high = ranges.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (ranges[mid].to <= offset) low = mid + 1;
    else high = mid;
  }
  return low;
}

function outside<T extends Range>(ranges: T[], excluded: Range[]): T[] {
  const kept: T[] = [];
  for (const range of ranges) {
    let from = range.from;
    for (
      let i = firstEndingAfter(excluded, from);
      i < excluded.length && excluded[i].from < range.to;
      i++
    ) {
      if (excluded[i].from > from) kept.push({ ...range, from, to: excluded[i].from });
      from = excluded[i].to;
    }
    if (range.to > from) kept.push({ ...range, from });
  }
  return kept;
}

/** Lines too long to render token by token become a single unhighlighted token. */
export function withLongLinesUntokenized(text: string, analysis: SourceAnalysis): SourceAnalysis {
  const lines = longLines(text);
  if (!lines.length) return analysis;
  return {
    tokens: [...outside(analysis.tokens, lines), ...lines].sort((a, b) => a.from - b.from),
    highlights: outside(analysis.highlights, lines),
  };
}

export const plainDiffAnalysis = (text: string) =>
  withLongLinesUntokenized(text, plainAnalysis(text));

export const analyzeDiffSource = async (path: string, text: string) =>
  withLongLinesUntokenized(text, await analyzeSource(path, text));
