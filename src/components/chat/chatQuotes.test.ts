import { describe, expect, it } from "vitest";
import { chatQuote, quoteExcerpt, withQuotes } from "./chatQuotes";

describe("withQuotes", () => {
  it("puts each quote before the message as a blockquote from the agent's reply", () => {
    const quotes = [chatQuote("a1", "First line\n\nSecond line"), chatQuote("a2", "Other")];
    expect(withQuotes("Why?", quotes)).toBe(
      "From your earlier reply:\n> First line\n>\n> Second line\n\nFrom your earlier reply:\n> Other\n\nWhy?"
    );
  });

  it("sends the quotes alone when the comment is empty", () => {
    expect(withQuotes("", [chatQuote("a1", "Only this")])).toBe(
      "From your earlier reply:\n> Only this"
    );
  });

  it("attributes a quote to the other agent that wrote the reply", () => {
    expect(withQuotes("", [chatQuote("a1", "Plan", "Codex")])).toBe(
      "From the earlier Codex reply:\n> Plan"
    );
  });

  it("leaves a message without quotes as it is", () => {
    expect(withQuotes("Hello", [])).toBe("Hello");
  });
});

describe("quoteExcerpt", () => {
  it("flattens whitespace", () => {
    expect(quoteExcerpt(chatQuote("a", "  one\n\ttwo  "))).toBe("one two");
  });

  it("shortens long quotes", () => {
    const excerpt = quoteExcerpt(chatQuote("a", "x".repeat(200)));
    expect(excerpt).toHaveLength(80);
    expect(excerpt.endsWith("…")).toBe(true);
  });
});
