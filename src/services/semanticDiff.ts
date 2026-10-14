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

interface Source {
  text: string;
  lines: string[];
  starts: number[];
  analysis: SourceAnalysis;
}

function source(text: string, analysis: SourceAnalysis): Source {
  const lines = text.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  const starts: number[] = [];
  let offset = 0;
  for (const line of lines) {
    starts.push(offset);
    offset += line.length + 1;
  }
  return { text, lines: text ? lines : [], starts, analysis };
}

function line(src: Source, index: number, kind: DiffLine["kind"], side: Side): DiffLine {
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

function tokensOf(src: Source, lines: DiffLine[]): Range[] {
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

function markTokenChanges(oldSrc: Source, newSrc: Source, removed: DiffLine[], added: DiffLine[]) {
  const oldTokens = tokensOf(oldSrc, removed);
  const newTokens = tokensOf(newSrc, added);
  if (oldTokens.length + newTokens.length > MAX_TOKENS_PER_CHANGE) return false;
  const text = (src: Source) => (range: Range) => src.text.slice(range.from, range.to);
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

function markMoves(groups: { removed: DiffLine[]; added: DiffLine[] }[]) {
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
 * Line diff refined by a token diff inside each changed region, so only the tokens that changed are
 * highlighted, lines whose tokens all survive are marked as formatting-only, and whole lines that
 * reappear elsewhere are marked as moved. Returns null when the line diff times out.
 */
export function semanticDiff(
  oldText: string,
  newText: string,
  oldAnalysis: SourceAnalysis = plainAnalysis(oldText),
  newAnalysis: SourceAnalysis = plainAnalysis(newText)
): DiffLine[] | null {
  const oldSrc = source(oldText, oldAnalysis);
  const newSrc = source(newText, newAnalysis);
  const changes = diffArrays(oldSrc.lines, newSrc.lines, { timeout: LINE_DIFF_TIMEOUT_MS });
  if (!changes) return null;
  const lines: DiffLine[] = [];
  const groups: { removed: DiffLine[]; added: DiffLine[] }[] = [];
  let oldIndex = 0;
  let newIndex = 0;
  let group: { removed: DiffLine[]; added: DiffLine[] } | null = null;
  for (const change of changes) {
    for (let i = 0; i < change.count; i++) {
      if (!change.added && !change.removed) {
        group = null;
        const context = line(newSrc, newIndex, "context", "new");
        context.oldNumber = ++oldIndex;
        context.newNumber = ++newIndex;
        lines.push(context);
        continue;
      }
      if (!group) {
        group = { removed: [], added: [] };
        groups.push(group);
      }
      const diffLine = change.added
        ? line(newSrc, newIndex, "added", "new")
        : line(oldSrc, oldIndex, "removed", "old");
      if (change.added) {
        diffLine.newNumber = ++newIndex;
        group.added.push(diffLine);
      } else {
        diffLine.oldNumber = ++oldIndex;
        group.removed.push(diffLine);
      }
      lines.push(diffLine);
    }
  }
  markMoves(groups);
  for (const { removed, added } of groups) {
    const pairedRemoved = removed.filter((diffLine) => !diffLine.moved);
    const pairedAdded = added.filter((diffLine) => !diffLine.moved);
    if (pairedRemoved.length && pairedAdded.length) {
      if (markTokenChanges(oldSrc, newSrc, pairedRemoved, pairedAdded)) continue;
    }
    for (const diffLine of [...pairedRemoved, ...pairedAdded]) {
      diffLine.formatting = diffLine.text.trim() === "";
    }
  }
  return lines;
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
