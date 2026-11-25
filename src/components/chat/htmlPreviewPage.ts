import type { ExtraProps } from "react-markdown";
import { THEME_COLOR_TOKENS } from "../../themes";

type HastElement = NonNullable<ExtraProps["node"]>;
type HastChild = HastElement["children"][number];

/** No sandbox flags: the page runs no scripts, has an opaque origin and cannot navigate the app. */
export const HTML_PREVIEW_SANDBOX = "";
export const HTML_PREVIEW_CSP =
  "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; media-src data:";
export const HTML_PREVIEW_PROPERTY = "dataHtmlPreview";

const OPENING_FENCE = /^[ \t>]*(`{3,}|~{3,})/;
const CLOSING_FENCE = /\n[ \t>]*(`{3,}|~{3,})[ \t]*\n?$/;

function isClosedFence(fence: string) {
  const opening = OPENING_FENCE.exec(fence)?.[1];
  const closing = CLOSING_FENCE.exec(fence)?.[1];
  return !!opening && !!closing && closing[0] === opening[0] && closing.length >= opening.length;
}

const isHtmlPreviewCode = (node: HastChild): node is HastElement =>
  node.type === "element" &&
  node.tagName === "code" &&
  String(node.properties.className).split(/[ ,]/).includes("language-html") &&
  String((node.data as { meta?: string } | undefined)?.meta ?? "")
    .split(/\s+/)
    .includes("preview");

function markHtmlPreviews(parent: { children: HastChild[] }, text: string) {
  for (const child of parent.children) {
    if (child.type !== "element") continue;
    if (child.tagName !== "pre") {
      markHtmlPreviews(child, text);
      continue;
    }
    if (!child.children.some(isHtmlPreviewCode)) continue;
    const fence = text.slice(child.position?.start.offset, child.position?.end.offset);
    child.properties[HTML_PREVIEW_PROPERTY] = isClosedFence(fence) ? "ready" : "streaming";
  }
}

/** Marks ```html preview fences, and whether they are closed so a streaming page isn't reloaded. */
export function rehypeHtmlPreview() {
  return (tree: { children: HastChild[] }, file: { value: unknown }) =>
    markHtmlPreviews(tree, String(file.value));
}

function themeStyle() {
  const root = document.documentElement;
  const computed = getComputedStyle(root);
  const colors = THEME_COLOR_TOKENS.map(({ key }) => {
    const value = computed.getPropertyValue(`--color-${key}`).trim();
    return value ? `--color-${key}: ${value};` : "";
  }).join(" ");
  const scheme = root.dataset.appearance === "light" ? "light" : "dark";
  return `:root { ${colors} color-scheme: ${scheme}; }
html { background: var(--color-bg-secondary, Canvas); color: var(--color-text-primary, CanvasText); font: 14px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", "Helvetica Neue", sans-serif; }
body { margin: 1rem; }
a { color: var(--color-accent, LinkText); }`;
}

export const htmlPreviewDocument = (source: string) =>
  `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${HTML_PREVIEW_CSP}"><style>${themeStyle()}</style></head><body>${source}</body></html>`;
