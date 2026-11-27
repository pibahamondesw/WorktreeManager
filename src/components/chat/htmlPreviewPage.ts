import type { ExtraProps } from "react-markdown";
import { THEME_COLOR_TOKENS } from "../../themes";
import { IS_WINDOWS, customProtocolUrl } from "../../platform";

type HastElement = NonNullable<ExtraProps["node"]>;
type HastChild = HastElement["children"][number];

const HTML_PREVIEW_SCHEME = "wtm-preview";
export const STATIC_HTML_PREVIEW_CSP =
  "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; media-src data:";
export const HTML_PREVIEW_PROPERTY = "dataHtmlPreview";
/** Posted by the preview protocol's bootstrap (src-tauri/src/html_preview.rs) on Escape. */
export const HTML_PREVIEW_ESCAPE_MESSAGE = "wtm-preview:escape";

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

const htmlPreviewDocument = (source: string, head = "") =>
  `<!doctype html><html><head><meta charset="utf-8">${head}<style>${themeStyle()}</style></head><body>${source}</body></html>`;

function base64Url(text: string) {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export type HtmlPreviewFrameSource =
  | { sandbox: "allow-scripts"; src: string }
  | { sandbox: ""; srcDoc: string };

/**
 * With scripts, the page loads from the `wtm-preview` protocol, whose CSP it gets
 * (src-tauri/src/html_preview.rs). Without allow-same-origin its origin is opaque, so it can't
 * read the app, navigate it or open windows; it travels in the fragment, which the protocol never
 * receives, so its size is unbounded.
 *
 * Windows stays static: WebView2 injects Tauri's initialization scripts, invoke key included,
 * into every frame, so a page with scripts there could call the app's commands.
 */
export function htmlPreviewFrameSource(
  source: string,
  windows = IS_WINDOWS
): HtmlPreviewFrameSource {
  if (windows)
    return {
      sandbox: "",
      srcDoc: htmlPreviewDocument(
        source,
        `<meta http-equiv="Content-Security-Policy" content="${STATIC_HTML_PREVIEW_CSP}">`
      ),
    };
  const page = base64Url(htmlPreviewDocument(source));
  return { sandbox: "allow-scripts", src: `${customProtocolUrl(HTML_PREVIEW_SCHEME)}#${page}` };
}
