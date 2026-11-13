import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { splitMarkdownBlocks } from "./markdownBlocks";

const renderWhole = (text: string) =>
  renderToStaticMarkup(<Markdown remarkPlugins={[remarkGfm]}>{text}</Markdown>);
const withoutBlockSeparators = (html: string) => html.replace(/>\n</g, "><");
const renderBlocks = (blocks: string[]) =>
  blocks
    .map((block) => renderToStaticMarkup(<Markdown remarkPlugins={[remarkGfm]}>{block}</Markdown>))
    .join("");

describe("splitMarkdownBlocks", () => {
  it("cuts at blank lines between top-level blocks", () => {
    expect(splitMarkdownBlocks("# Title\n\nFirst\nline\n\n> a\n> b")).toEqual([
      "# Title\n\n",
      "First\nline\n\n",
      "> a\n> b",
    ]);
  });

  it.each([
    ["a fenced code block with blank lines", "```ts\nconst a = 1;\n\nconst b = 2;\n```\n\nAfter"],
    ["a tilde fence", "~~~\none\n\n```\ntwo\n~~~\n\nAfter"],
    ["a loose list", "- one\n\n- two\n\n  continued\n\nAfter"],
    ["an ordered list", "1. one\n\n2. two\n\nAfter"],
    ["indented code", "    one\n\n    two\n\nAfter"],
    ["a table", "| a | b |\n| - | - |\n| 1 | 2 |\n\nAfter"],
  ])("keeps %s in one block", (_, text) => {
    const blocks = splitMarkdownBlocks(text)!;
    expect(blocks.at(-1)).toBe("After");
    expect(blocks).toHaveLength(2);
  });

  it("keeps an unclosed fence open until the end while streaming", () => {
    expect(splitMarkdownBlocks("Intro\n\n```\ncode\n\nmore")).toEqual([
      "Intro\n\n",
      "```\ncode\n\nmore",
    ]);
  });

  it("stops cutting when content leaves an indented fence", () => {
    expect(splitMarkdownBlocks("Intro\n\nList:\n- item\n  ```\n  code\nout\n\nNext")).toEqual([
      "Intro\n\n",
      "List:\n- item\n  ```\n  code\nout\n\nNext",
    ]);
  });

  it.each([
    ["link definitions", "See [docs][d]\n\n[d]: https://example.com"],
    ["a link definition in a quote", "See [docs][d]\n\n> [d]: https://example.com"],
    ["a link definition in a list", "See [docs][d]\n\n- [d]: https://example.com"],
    ["a multi-line link label", "See [foo bar]\n\n[foo\nbar]: https://example.com"],
    ["footnotes", "Claim[^1]\n\n[^1]: Source"],
    ["raw HTML", "<details>\n\nBody\n\n</details>"],
    ["carriage returns", "One\r\n\r\nTwo"],
  ])("falls back to the whole text with %s", (_, text) => {
    expect(splitMarkdownBlocks(text)).toBeNull();
  });

  it.each([
    "# Title\n\nParagraph with `code` and **bold**.\n\n- a\n- b\n\n1. one\n\n2. two",
    "Text\n\n```js\nlet a;\n\nlet b;\n```\n\n> quote\n\n| a |\n| - |\n| 1 |\n\n---\n\nEnd",
    "- item\n\n  nested paragraph\n\n      indented code\n\nAfter",
    "```npm install``` runs it.\n\n```\nfoo\n\nbar\n```\n\nEnd",
  ])("renders the same as the whole text: %#", (text) => {
    expect(withoutBlockSeparators(renderBlocks(splitMarkdownBlocks(text)!))).toBe(
      withoutBlockSeparators(renderWhole(text))
    );
  });
});
