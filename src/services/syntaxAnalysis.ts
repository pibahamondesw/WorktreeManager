import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { highlightTree, tagHighlighter, tags } from "@lezer/highlight";
import { plainAnalysis, Range, SourceAnalysis, StyledRange, wordTokens } from "./semanticDiff";

type Tree = Parameters<typeof highlightTree>[0];

const MAX_PARSED_LENGTH = 1_000_000;

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
