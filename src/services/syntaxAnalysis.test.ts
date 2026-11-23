import { describe, expect, it } from "vitest";
import { analyzeDiffSource, plainDiffAnalysis, withLongLinesUntokenized } from "./syntaxAnalysis";

const longLine = `const value = "${"x".repeat(1000)}";`;
const slice = (text: string) => (range: { from: number; to: number }) =>
  text.slice(range.from, range.to);

describe("withLongLinesUntokenized", () => {
  it("keeps analyses without long lines untouched", () => {
    const analysis = plainDiffAnalysis("a b\nc");

    expect(withLongLinesUntokenized("a b\nc", analysis)).toBe(analysis);
  });

  it("turns each long line into one token without highlights", async () => {
    const text = `let a;\n${longLine}\nlet b;`;

    const analysis = await analyzeDiffSource("src/example.ts", text);

    expect(analysis.tokens.map(slice(text))).toEqual(["let", "a", ";", longLine, "let", "b", ";"]);
    expect(analysis.highlights.map(slice(text))).toEqual(["let", "a", ";", "let", "b", ";"]);
  });

  it("keeps the highlighted parts of ranges that cross a long line", () => {
    const text = `/*\n${longLine}\n*/`;
    const comment = { from: 0, to: text.length, className: "hljs-comment" };

    const analysis = withLongLinesUntokenized(text, { tokens: [], highlights: [comment] });

    expect(analysis.highlights.map(slice(text))).toEqual(["/*\n", "\n*/"]);
    expect(analysis.tokens.map(slice(text))).toEqual([longLine]);
  });
});
