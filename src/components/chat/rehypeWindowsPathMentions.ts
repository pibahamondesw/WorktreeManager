import type { ExtraProps } from "react-markdown";
import { windowsPathMentionHref } from "./markdownDocument";

type HastChild = NonNullable<ExtraProps["node"]>["children"][number];

function linkPathMentions(parent: { children: HastChild[] }) {
  for (const [index, child] of parent.children.entries()) {
    if (child.type !== "element" || child.tagName === "a" || child.tagName === "pre") continue;
    if (child.tagName !== "code") {
      linkPathMentions(child);
      continue;
    }
    const [text] = child.children;
    if (child.properties.className || text?.type !== "text") continue;
    const href = windowsPathMentionHref(text.value);
    if (!href) continue;
    parent.children[index] = {
      type: "element",
      tagName: "a",
      properties: { href },
      children: [child],
    };
  }
}

export function rehypeWindowsPathMentions() {
  return linkPathMentions;
}
