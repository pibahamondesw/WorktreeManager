// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { quoteRange } from "./useQuoteNavigation";

const rowWith = (html: string) => {
  const row = document.createElement("div");
  row.innerHTML = `<button>First</button><div data-find-text>${html}</div>`;
  return row;
};

describe("quoteRange", () => {
  it("finds the quoted text in the rendered reply", () => {
    const row = rowWith("<p>Keep the <strong>first</strong> step</p>");
    expect(quoteRange(row, "the first step")?.toString()).toBe("the first step");
  });

  it("falls back to the first line when the quote spans paragraphs", () => {
    const row = rowWith("<p>One plan.</p><p>Two steps.</p>");
    expect(quoteRange(row, "One plan.\n\nTwo steps.")?.toString()).toBe("One plan.");
  });

  it("finds nothing when the reply no longer has the text", () => {
    expect(quoteRange(rowWith("<p>Rewritten</p>"), "Original")).toBeNull();
  });
});
