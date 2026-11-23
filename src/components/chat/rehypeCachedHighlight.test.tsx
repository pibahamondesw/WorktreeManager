import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import Markdown, { ExtraProps } from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import { rehypeCachedHighlight } from "./rehypeCachedHighlight";

type HastElement = NonNullable<ExtraProps["node"]>;

const PLAIN_TEXT = ["mermaid", "math"];

const renderWith = (rehypePlugins: Parameters<typeof Markdown>[0]["rehypePlugins"], text: string) =>
  renderToStaticMarkup(<Markdown rehypePlugins={rehypePlugins}>{text}</Markdown>);

const cachedHtml = (text: string) => renderWith([rehypeCachedHighlight], text);
const uncachedHtml = (text: string) =>
  renderWith([[rehypeHighlight, { plainText: PLAIN_TEXT }]], text);

function highlightedNode(text: string) {
  let node: HastElement | undefined;
  renderToStaticMarkup(
    <Markdown
      rehypePlugins={[rehypeCachedHighlight]}
      components={{
        code: (props) => {
          node = props.node;
          return null;
        },
      }}
    >
      {text}
    </Markdown>
  );
  return node!;
}

describe("rehypeCachedHighlight", () => {
  it.each([
    ["a known language", "```ts\nconst first = 1;\n```"],
    ["a nested block", "> - item\n>\n>   ```js\n>   let nested = true;\n>   ```"],
    ["plain-text languages", "```mermaid\ngraph TD; A-->B\n```"],
    ["an unknown language", "```not-a-language\nraw text\n```"],
    ["no language", "```\nplain block\n```"],
    ["inline code", "Use `code` here"],
  ])("renders %s like rehype-highlight", (_, text) => {
    expect(cachedHtml(text)).toBe(uncachedHtml(text));
  });

  it("reuses the highlighted result for a block it has seen", () => {
    const text = "```py\nreused = True\n```\n\nOutro";
    const first = highlightedNode(text);

    const second = highlightedNode(`Intro\n\n${text}`);

    expect(second).not.toBe(first);
    expect(second.children).toBe(first.children);
    expect(second.properties.className).toEqual(["hljs", "language-py"]);
  });

  it("doesn't cache a block that ends the text, since it may still grow", () => {
    const growing = highlightedNode("```ts\nconst growing = 1");

    const closed = highlightedNode("```ts\nconst growing = 1\n```\n\nAfter");
    const again = highlightedNode("```ts\nconst growing = 1\n```\n\nLater");

    expect(closed.children).not.toBe(growing.children);
    expect(again.children).toBe(closed.children);
  });

  it("highlights the same code separately per language", () => {
    const python = highlightedNode("```py\nvalue = 1\n```");

    const plain = highlightedNode("```\nvalue = 1\n```");

    expect(plain.children).not.toBe(python.children);
    expect(plain.properties.className).toBeUndefined();
  });
});
