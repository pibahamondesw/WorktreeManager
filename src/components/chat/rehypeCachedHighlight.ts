import type { ExtraProps } from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import { unified } from "unified";
import { LruCache } from "./lruCache";

type HastElement = NonNullable<ExtraProps["node"]>;
type HastChild = HastElement["children"][number];
type HighlightedCode = Pick<HastElement, "children"> & {
  className: HastElement["properties"]["className"];
};

const MAX_HIGHLIGHTED_BLOCKS = 500;
const MAX_HIGHLIGHTED_CHARACTERS = 300_000;

const highlightedCode = new LruCache<HighlightedCode>(
  MAX_HIGHLIGHTED_BLOCKS,
  MAX_HIGHLIGHTED_CHARACTERS
);
const highlighter = unified().use(rehypeHighlight, { plainText: ["mermaid", "math"] });

const textContent = (node: HastChild): string =>
  node.type === "text"
    ? node.value
    : node.type === "element"
      ? node.children.map(textContent).join("")
      : "";

function highlight(code: HastElement, pre: HastElement): HighlightedCode {
  highlighter.runSync({ type: "root", children: [{ ...pre, children: [code] }] });
  return { className: code.properties.className, children: code.children };
}

const endsAt = (node: HastElement, length: number) => node.position?.end.offset === length;

function highlightCodeBlocks(parent: { children: HastChild[] }, documentLength: number) {
  for (const child of parent.children) {
    if (child.type !== "element") continue;
    if (child.tagName !== "pre") {
      highlightCodeBlocks(child, documentLength);
      continue;
    }
    const mayStillGrow = endsAt(child, documentLength);
    for (const code of child.children) {
      if (code.type !== "element" || code.tagName !== "code") continue;
      const text = textContent(code);
      const key = `${String(code.properties.className)}\n${text}`;
      let highlighted = highlightedCode.get(key);
      if (!highlighted) {
        highlighted = highlight(code, child);
        if (!mayStillGrow) highlightedCode.set(key, highlighted, text.length);
      }
      code.properties.className = highlighted.className;
      code.children = highlighted.children;
    }
  }
}

/**
 * rehype-highlight that reuses the result for code blocks it has already highlighted. A block
 * that ends the document isn't cached, since streaming may still extend it.
 */
export function rehypeCachedHighlight() {
  return (tree: { children: HastChild[] }, file: { value: unknown }) =>
    highlightCodeBlocks(tree, String(file.value).length);
}
