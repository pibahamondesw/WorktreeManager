// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { ChatItem } from "../../services/chat";
import { chatEntries } from "./chatEntries";
import { findMatches, textRanges } from "./chatFind";

const item = (id: string, kind: ChatItem["kind"], text: string, extra = {}): ChatItem => ({
  id,
  kind,
  text,
  ...extra,
});

describe("findMatches", () => {
  it("finds every occurrence in messages, ignoring case and tool activity", () => {
    const rows = chatEntries([
      item("u", "user", "Where is the Bug? bug!"),
      item("c", "command", "grep bug"),
      item("r", "reasoning", "the bug"),
      item("a", "assistant", "No BUG here"),
    ]);
    expect(findMatches(rows, "bug")).toEqual([
      { rowKey: "u", occurrence: 0 },
      { rowKey: "u", occurrence: 1 },
      { rowKey: "a", occurrence: 0 },
    ]);
  });

  it("finds nothing for an empty query", () => {
    expect(findMatches(chatEntries([item("u", "user", "text")]), "")).toEqual([]);
  });

  it("treats the query as literal text", () => {
    expect(findMatches(chatEntries([item("u", "user", "a.b axb (c)")]), "a.b")).toHaveLength(1);
    expect(findMatches(chatEntries([item("u", "user", "a.b axb (c)")]), "(c)")).toHaveLength(1);
  });

  it("counts occurrences without overlapping them", () => {
    expect(findMatches(chatEntries([item("u", "user", "aaaa")]), "aa")).toHaveLength(2);
  });
});

describe("textRanges", () => {
  it("matches text split across elements and skips controls", () => {
    const root = document.createElement("div");
    root.innerHTML =
      "<p>Fix the <strong>bu</strong>g and the BUG</p><button>bug</button><span class='sr-only'>bug</span>";
    const ranges = textRanges(root, "bug");
    expect(ranges.map((range) => range.toString())).toEqual(["bug", "BUG"]);
    expect(ranges[0].startContainer.textContent).toBe("bu");
    expect(ranges[0].endContainer.textContent).toBe("g and the BUG");
  });

  it("keeps offsets right after characters that lengthen when lowercased", () => {
    const root = document.createElement("p");
    root.textContent = "İstanbul abc";
    expect(textRanges(root, "abc").map((range) => range.toString())).toEqual(["abc"]);
  });

  it("finds nothing for an empty query", () => {
    const root = document.createElement("div");
    root.textContent = "text";
    expect(textRanges(root, "")).toEqual([]);
  });
});
