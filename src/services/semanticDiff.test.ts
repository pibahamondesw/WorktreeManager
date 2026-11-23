import { describe, expect, it } from "vitest";
import {
  diffBlocks,
  DiffLine,
  lineDiff,
  lineSegments,
  refineByTokens,
  semanticDiff,
  SourceAnalysis,
} from "./semanticDiff";
import { analyzeSource } from "./syntaxAnalysis";

const changed = (line: DiffLine) =>
  line.changes.map((range) => line.text.slice(range.from, range.to));
const summary = (lines: DiffLine[]) =>
  lines.map((line) => [line.kind, line.text, line.formatting, line.moved]);

describe("semanticDiff", () => {
  it("highlights only the tokens that changed inside a modified line", () => {
    const lines = semanticDiff(
      "const total = price * qty;\nreturn total;\n",
      "const total = price * quantity;\nreturn total;\n"
    )!;

    expect(lines.map((line) => [line.kind, line.oldNumber, line.newNumber])).toEqual([
      ["removed", 1, undefined],
      ["added", undefined, 1],
      ["context", 2, 2],
    ]);
    expect(changed(lines[0])).toEqual(["qty"]);
    expect(changed(lines[1])).toEqual(["quantity"]);
    expect(lines[0].formatting).toBe(false);
  });

  it("marks reflowed code as formatting-only and hides it on request", () => {
    const lines = semanticDiff("call(a, b);\n", "call(\n  a,\n  b\n);\n")!;

    expect(lines.every((line) => line.formatting)).toBe(true);
    expect(diffBlocks(lines, false).some((block) => block.kind === "lines")).toBe(true);
    const hidden = diffBlocks(lines, true);
    expect(hidden.every((block) => block.kind === "gap")).toBe(true);
    expect(hidden.flatMap((block) => block.lines).map((line) => line.kind)).toEqual(
      Array(4).fill("context")
    );
  });

  it("marks whole lines that move elsewhere instead of diffing their tokens", () => {
    const lines = semanticDiff(
      "const moved = compute();\nfirst();\nsecond();\nthird();\n",
      "first();\nsecond();\nthird();\nconst moved = compute();\n"
    )!;

    expect(summary(lines.filter((line) => line.kind !== "context"))).toEqual([
      ["removed", "const moved = compute();", false, true],
      ["added", "const moved = compute();", false, true],
    ]);
  });

  it("treats pure additions as changes and blank lines as formatting", () => {
    const lines = semanticDiff("", "one\n\ntwo\n")!;

    expect(summary(lines)).toEqual([
      ["added", "one", false, false],
      ["added", "", true, false],
      ["added", "two", false, false],
    ]);
  });
});

describe("refineByTokens", () => {
  const before = "const moved = compute();\nconst total = price * qty;\nend();\n";
  const after = "const total = price * quantity;\nend();\nconst moved = compute();\n";
  const wholeLines = (text: string): SourceAnalysis => {
    let from = 0;
    const tokens = text.split("\n").map((line) => {
      const range = { from, to: from + line.length };
      from = range.to + 1;
      return range;
    });
    return { tokens, highlights: [] };
  };

  it("refines a copy of the line diff and keeps its moved lines", () => {
    const lines = lineDiff(before, after)!;

    const refined = refineByTokens(lines, before, after);

    expect(refined.filter((line) => line.moved).map((line) => line.kind)).toEqual([
      "removed",
      "added",
    ]);
    expect(refined.filter((line) => line.changes.length).map(changed)).toEqual([
      ["qty"],
      ["quantity"],
    ]);
    expect(lines.every((line) => line.changes.length === 0 && !line.formatting)).toBe(true);
  });

  it("replaces a previous refinement with the one from a new analysis", () => {
    const words = refineByTokens(lineDiff(before, after)!, before, after);

    const refined = refineByTokens(words, before, after, wholeLines(before), wholeLines(after));

    expect(refined.filter((line) => line.changes.length).map(changed)).toEqual([
      ["const total = price * qty;"],
      ["const total = price * quantity;"],
    ]);
  });

  it("clears previous token changes when the new token diff is too large", () => {
    const longBefore = Array.from({ length: 7000 }, (_, i) => `a${i}`).join(" ");
    const longAfter = `${longBefore} b`;
    const lines = lineDiff(longBefore, longAfter)!;
    const analyses = [wholeLines(longBefore), wholeLines(longAfter)] as const;
    const byLines = refineByTokens(lines, longBefore, longAfter, ...analyses);

    const refined = refineByTokens(byLines, longBefore, longAfter);

    const marks = (diff: DiffLine[]) => diff.map((line) => [line.changes.length, line.formatting]);
    expect(marks(byLines)).toEqual([
      [1, false],
      [1, false],
    ]);
    expect(marks(refined)).toEqual([
      [0, false],
      [0, false],
    ]);
  });
});

describe("diffBlocks", () => {
  it("keeps three lines of context around changes and collapses the rest", () => {
    const old = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`);
    const next = old.map((line, i) => (i === 9 ? "line ten" : line));

    const blocks = diffBlocks(semanticDiff(old.join("\n"), next.join("\n"))!, false);

    expect(blocks.map((block) => [block.kind, block.lines.length])).toEqual([
      ["gap", 6],
      ["lines", 8],
      ["gap", 7],
    ]);
  });
});

describe("lineSegments", () => {
  it("splits a line at syntax and change boundaries", () => {
    const [removed] = semanticDiff("let value = 1;", "let value = 2;")!;

    expect(
      lineSegments(removed, [
        { from: 0, to: 3, className: "hljs-keyword" },
        { from: 12, to: 13, className: "hljs-number" },
      ])
    ).toEqual([
      { text: "let", className: "hljs-keyword", changed: false },
      { text: " value = ", className: undefined, changed: false },
      { text: "1", className: "hljs-number", changed: true },
      { text: ";", className: undefined, changed: false },
    ]);
  });
});

describe("analyzeSource", () => {
  it("uses the language grammar for tokens and highlighting", async () => {
    const text = 'const a => "two words";';

    const analysis = await analyzeSource("src/example.ts", text);

    const tokens = analysis.tokens.map((range) => text.slice(range.from, range.to));
    expect(tokens).toEqual(["const", "a", "=>", '"', "two", "words", '"', ";"]);
    expect(analysis.highlights).toContainEqual({ from: 0, to: 5, className: "hljs-keyword" });
  });

  it("falls back to word tokens for unknown files", async () => {
    const analysis = await analyzeSource("notes.unknown-extension", "a-b");

    expect(analysis.highlights).toEqual([]);
    expect(analysis.tokens).toHaveLength(3);
  });
});
