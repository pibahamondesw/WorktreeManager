import { diffArrays } from "diff";

export interface Range {
  from: number;
  to: number;
}

export interface StyledRange extends Range {
  className: string;
}

export interface SourceAnalysis {
  tokens: Range[];
  highlights: StyledRange[];
}

type Side = "old" | "new";

export interface DiffLine {
  kind: "context" | "added" | "removed";
  side: Side;
  start: number;
  text: string;
  oldNumber?: number;
  newNumber?: number;
  changes: Range[];
  formatting: boolean;
  moved: boolean;
}

export type DiffBlock = { kind: "lines"; lines: DiffLine[] } | { kind: "gap"; lines: DiffLine[] };

export interface Segment {
  text: string;
  className?: string;
  changed: boolean;
}

const LINE_DIFF_TIMEOUT_MS = 1500;
const TOKEN_DIFF_TIMEOUT_MS = 300;
const MAX_TOKENS_PER_CHANGE = 6000;
const MIN_MOVED_LINE_LENGTH = 8;
const WORD = /[\p{L}\p{N}_$]+|[^\p{L}\p{N}_$\s]/gu;

export function wordTokens(text: string, from = 0, to = text.length): Range[] {
  const tokens: Range[] = [];
  for (const match of text.slice(from, to).matchAll(WORD)) {
    const start = from + (match.index ?? 0);
    tokens.push({ from: start, to: start + match[0].length });
  }
  return tokens;
}

export const plainAnalysis = (text: string): SourceAnalysis => ({
  tokens: wordTokens(text),
  highlights: [],
});

interface SourceLines {
  lines: string[];
  starts: number[];
}

interface AnalyzedSource {
  text: string;
  analysis: SourceAnalysis;
}

interface ChangeGroup {
  removed: DiffLine[];
  added: DiffLine[];
}

function sourceLines(text: string): SourceLines {
  const lines = text.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  const starts: number[] = [];
  let offset = 0;
  for (const line of lines) {
    starts.push(offset);
    offset += line.length + 1;
  }
  return { lines: text ? lines : [], starts };
}

function line(src: SourceLines, index: number, kind: DiffLine["kind"], side: Side): DiffLine {
  return {
    kind,
    side,
    start: src.starts[index],
    text: src.lines[index],
    changes: [],
    formatting: false,
    moved: false,
  };
}

const normalized = (text: string) => text.replace(/\s+/g, " ").trim();

function firstAtOrAfter(ranges: Range[], offset: number): number {
  let low = 0;
  let high = ranges.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (ranges[mid].to <= offset) low = mid + 1;
    else high = mid;
  }
  return low;
}

function tokensOf(src: AnalyzedSource, lines: DiffLine[]): Range[] {
  const tokens: Range[] = [];
  for (const diffLine of lines) {
    const end = diffLine.start + diffLine.text.length;
    const all = src.analysis.tokens;
    for (let i = firstAtOrAfter(all, diffLine.start); i < all.length && all[i].from < end; i++) {
      tokens.push({ from: Math.max(all[i].from, diffLine.start), to: Math.min(all[i].to, end) });
    }
  }
  return tokens;
}

function markTokenChanges(
  oldSrc: AnalyzedSource,
  newSrc: AnalyzedSource,
  removed: DiffLine[],
  added: DiffLine[]
) {
  const oldTokens = tokensOf(oldSrc, removed);
  const newTokens = tokensOf(newSrc, added);
  if (oldTokens.length + newTokens.length > MAX_TOKENS_PER_CHANGE) return false;
  const text = (src: AnalyzedSource) => (range: Range) => src.text.slice(range.from, range.to);
  const changes = diffArrays(oldTokens.map(text(oldSrc)), newTokens.map(text(newSrc)), {
    timeout: TOKEN_DIFF_TIMEOUT_MS,
  });
  if (!changes) return false;
  const changedOld: Range[] = [];
  const changedNew: Range[] = [];
  let oldIndex = 0;
  let newIndex = 0;
  for (const change of changes) {
    if (change.removed) changedOld.push(...oldTokens.slice(oldIndex, oldIndex + change.count));
    if (change.added) changedNew.push(...newTokens.slice(newIndex, newIndex + change.count));
    if (!change.added) oldIndex += change.count;
    if (!change.removed) newIndex += change.count;
  }
  for (const [lines, changed] of [
    [removed, changedOld],
    [added, changedNew],
  ] as const) {
    for (const diffLine of lines) {
      const end = diffLine.start + diffLine.text.length;
      diffLine.changes = changed
        .filter((range) => range.from >= diffLine.start && range.to <= end)
        .map((range) => ({ from: range.from - diffLine.start, to: range.to - diffLine.start }));
      diffLine.formatting = diffLine.changes.length === 0;
    }
  }
  return true;
}

function changeGroups(lines: DiffLine[]): ChangeGroup[] {
  const groups: ChangeGroup[] = [];
  let group: ChangeGroup | null = null;
  for (const diffLine of lines) {
    if (diffLine.kind === "context") {
      group = null;
      continue;
    }
    if (!group) {
      group = { removed: [], added: [] };
      groups.push(group);
    }
    (diffLine.kind === "added" ? group.added : group.removed).push(diffLine);
  }
  return groups;
}

function markMoves(groups: ChangeGroup[]) {
  const removedByText = new Map<string, { line: DiffLine; group: number }[]>();
  groups.forEach((group, index) => {
    for (const diffLine of group.removed) {
      const key = normalized(diffLine.text);
      if (key.length < MIN_MOVED_LINE_LENGTH) continue;
      removedByText.set(key, [...(removedByText.get(key) ?? []), { line: diffLine, group: index }]);
    }
  });
  groups.forEach((group, index) => {
    for (const diffLine of group.added) {
      const candidates = removedByText.get(normalized(diffLine.text));
      const match = candidates?.findIndex((candidate) => candidate.group !== index) ?? -1;
      if (!candidates || match < 0) continue;
      candidates[match].line.moved = true;
      diffLine.moved = true;
      candidates.splice(match, 1);
    }
  });
}

/**
 * Line diff with whole lines that reappear elsewhere marked as moved, independent of any source
 * analysis. Returns null when it times out.
 */
export function lineDiff(oldText: string, newText: string): DiffLine[] | null {
  const oldSrc = sourceLines(oldText);
  const newSrc = sourceLines(newText);
  const changes = diffArrays(oldSrc.lines, newSrc.lines, { timeout: LINE_DIFF_TIMEOUT_MS });
  if (!changes) return null;
  const lines: DiffLine[] = [];
  let oldIndex = 0;
  let newIndex = 0;
  for (const change of changes) {
    for (let i = 0; i < change.count; i++) {
      if (change.added) {
        const added = line(newSrc, newIndex, "added", "new");
        added.newNumber = ++newIndex;
        lines.push(added);
      } else if (change.removed) {
        const removed = line(oldSrc, oldIndex, "removed", "old");
        removed.oldNumber = ++oldIndex;
        lines.push(removed);
      } else {
        const context = line(newSrc, newIndex, "context", "new");
        context.oldNumber = ++oldIndex;
        context.newNumber = ++newIndex;
        lines.push(context);
      }
    }
  }
  markMoves(changeGroups(lines));
  return lines;
}

/**
 * Copy of a line diff refined by a token diff inside each changed region, so only the tokens that
 * changed are highlighted and lines whose tokens all survive are marked as formatting-only. Any
 * previous refinement is discarded, so refining an already refined diff with a new analysis works.
 */
export function refineByTokens(
  lines: DiffLine[],
  oldText: string,
  newText: string,
  oldAnalysis: SourceAnalysis = plainAnalysis(oldText),
  newAnalysis: SourceAnalysis = plainAnalysis(newText)
): DiffLine[] {
  const refined = lines.map((diffLine) => ({ ...diffLine, changes: [], formatting: false }));
  const oldSrc = { text: oldText, analysis: oldAnalysis };
  const newSrc = { text: newText, analysis: newAnalysis };
  for (const { removed, added } of changeGroups(refined)) {
    const pairedRemoved = removed.filter((diffLine) => !diffLine.moved);
    const pairedAdded = added.filter((diffLine) => !diffLine.moved);
    if (pairedRemoved.length && pairedAdded.length) {
      if (markTokenChanges(oldSrc, newSrc, pairedRemoved, pairedAdded)) continue;
    }
    for (const diffLine of [...pairedRemoved, ...pairedAdded]) {
      diffLine.formatting = diffLine.text.trim() === "";
    }
  }
  return refined;
}

/** Line diff refined by tokens; null when the line diff times out. */
export function semanticDiff(
  oldText: string,
  newText: string,
  oldAnalysis?: SourceAnalysis,
  newAnalysis?: SourceAnalysis
): DiffLine[] | null {
  const lines = lineDiff(oldText, newText);
  return lines && refineByTokens(lines, oldText, newText, oldAnalysis, newAnalysis);
}

const isChange = (diffLine: DiffLine, hideFormatting: boolean) =>
  diffLine.kind !== "context" && !(hideFormatting && diffLine.formatting);

/**
 * Groups lines into visible hunks with surrounding context and collapsible gaps. With
 * hideFormatting, formatting-only removals disappear and formatting-only additions read as context.
 */
export function diffBlocks(lines: DiffLine[], hideFormatting: boolean, context = 3): DiffBlock[] {
  const visibleLines = hideFormatting
    ? lines
        .filter((diffLine) => !(diffLine.formatting && diffLine.kind === "removed"))
        .map((diffLine) =>
          diffLine.formatting && diffLine.kind === "added"
            ? { ...diffLine, kind: "context" as const }
            : diffLine
        )
    : lines;
  const near = new Array<boolean>(visibleLines.length).fill(false);
  visibleLines.forEach((diffLine, index) => {
    if (!isChange(diffLine, hideFormatting)) return;
    const end = Math.min(visibleLines.length - 1, index + context);
    for (let i = Math.max(0, index - context); i <= end; i++) near[i] = true;
  });
  const blocks: DiffBlock[] = [];
  visibleLines.forEach((diffLine, index) => {
    const kind = near[index] ? "lines" : "gap";
    const last = blocks[blocks.length - 1];
    if (last?.kind === kind) last.lines.push(diffLine);
    else blocks.push({ kind, lines: [diffLine] });
  });
  return blocks;
}

export function lineSegments(diffLine: DiffLine, highlights: StyledRange[]): Segment[] {
  const { start, text } = diffLine;
  const end = start + text.length;
  const boundaries = new Set([0, text.length]);
  const styles: StyledRange[] = [];
  for (let i = firstAtOrAfter(highlights, start); i < highlights.length; i++) {
    const range = highlights[i];
    if (range.from >= end) break;
    const from = Math.max(range.from, start) - start;
    const to = Math.min(range.to, end) - start;
    styles.push({ from, to, className: range.className });
    boundaries.add(from).add(to);
  }
  for (const range of diffLine.changes) boundaries.add(range.from).add(range.to);
  const points = [...boundaries].sort((a, b) => a - b);
  const segments: Segment[] = [];
  for (let i = 0; i < points.length - 1; i++) {
    const [from, to] = [points[i], points[i + 1]];
    const className = styles.find((style) => style.from <= from && style.to >= to)?.className;
    const changed = diffLine.changes.some((range) => range.from <= from && range.to >= to);
    const previous = segments[segments.length - 1];
    if (previous && previous.className === className && previous.changed === changed) {
      previous.text += text.slice(from, to);
    } else {
      segments.push({ text: text.slice(from, to), className, changed });
    }
  }
  return segments;
}
