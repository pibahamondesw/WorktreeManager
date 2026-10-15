import { ChatItem } from "../../services/chat";

const MARKDOWN_PATH = /\.(md|markdown|mdx)$/i;
const URL_SCHEME = /^[a-z][a-z0-9+.-]*:/i;
const LINE_SUFFIX = /(:\d+)+$/;
const FENCE = /^\s*(```|~~~)/;
const PREVIEW_LINES = 5;

export type MarkdownSource =
  | { kind: "file"; path: string; folders?: string[] }
  | { kind: "inline"; title: string; text: string };

export interface DocumentSummary {
  title: string | null;
  lines: string[];
}

export interface Frontmatter {
  entries: [string, string][];
  body: string;
}

const safeDecode = (value: string) => {
  try {
    return decodeURI(value);
  } catch {
    return value;
  }
};

export function markdownPath(reference: string | undefined): string | null {
  if (!reference) return null;
  let path = reference.trim().replace(/^<|>$/g, "");
  if (path.startsWith("file://")) path = path.slice("file://".length);
  else if (URL_SCHEME.test(path)) return null;
  path = safeDecode(path.split(/[?#]/)[0]).replace(LINE_SUFFIX, "");
  return MARKDOWN_PATH.test(path) ? path : null;
}

const withoutFencedCode = (text: string) => {
  let fenced = false;
  return text
    .split("\n")
    .filter((line) => {
      if (FENCE.test(line)) fenced = !fenced;
      else return !fenced;
      return false;
    })
    .join("\n");
};

export function markdownReferences(text: string): string[] {
  const prose = withoutFencedCode(text);
  const links = [...prose.matchAll(/\]\(\s*(<[^>]+>|[^)\s]+)/g)].map((match) => match[1]);
  const code = [...prose.matchAll(/`([^`\s]+)`/g)].map((match) => match[1]);
  const paths = [...links, ...code].map(markdownPath).filter((path) => path !== null);
  return [...new Set(paths)];
}

export function splitFrontmatter(text: string): Frontmatter {
  const match = /^---\r?\n([\s\S]*?)\r?\n---[ \t]*(\r?\n|$)/.exec(text);
  if (!match) return { entries: [], body: text };
  const entries = match[1]
    .split(/\r?\n/)
    .map((line) => /^([\w-]+):\s*(.*)$/.exec(line))
    .filter((entry) => entry !== null)
    .map(([, key, value]): [string, string] => [key, value.replace(/^["']|["']$/g, "")]);
  return { entries, body: text.slice(match[0].length) };
}

function plainInline(line: string) {
  return line
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/<[^>]+>/g, "")
    .replace(/(\*\*|__|~~|`)/g, "")
    .replace(/(^|\W)[*_](\S[^*_]*)[*_](?=\W|$)/g, "$1$2")
    .trim();
}

function plainLine(line: string) {
  const block = line
    .replace(/^\s*>+\s?/, "")
    .replace(/^\s*#{1,6}\s+/, "")
    .replace(/^\s*[-*+]\s+\[[ xX]\]\s+/, "")
    .replace(/^\s*[-*+]\s+/, "• ")
    .replace(/^\s*\|/, "")
    .replace(/\|\s*$/, "")
    .replace(/\s*\|\s*/g, " · ");
  return plainInline(block);
}

const isRule = (line: string) => /^\s*([-*_]\s*){3,}$/.test(line) || /^[\s|:-]+$/.test(line);

export function documentSummary(text: string): DocumentSummary {
  const lines = withoutFencedCode(splitFrontmatter(text).body.replace(/<!--[\s\S]*?-->/g, ""))
    .split("\n")
    .filter((line) => line.trim() && !isRule(line));
  const heading = lines.findIndex((line) => /^\s*#{1,6}\s+/.test(line));
  const title = heading === -1 ? null : plainLine(lines[heading]);
  return {
    title,
    lines: lines
      .filter((_, index) => index !== heading)
      .map(plainLine)
      .filter(Boolean)
      .slice(0, PREVIEW_LINES),
  };
}

export const fileName = (path: string) => path.slice(path.lastIndexOf("/") + 1);

export const directoryOf = (path: string) => path.slice(0, path.lastIndexOf("/")) || "/";

export function relativeTo(path: string, folders: string[]) {
  const folder = folders.find((candidate) => path.startsWith(`${candidate}/`));
  return folder ? path.slice(folder.length + 1) : path;
}

export const isPlanItem = (item: ChatItem) =>
  item.kind === "notice" && item.title === "Plan" && item.text.trim() !== "";
