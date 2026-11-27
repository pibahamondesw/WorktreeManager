import { ChatItem } from "../../services/chat";
import { isWindowsDrivePath, isWithinPath } from "../../platform";

const MARKDOWN_PATH = /\.(md|markdown|mdx)$/i;
const URL_SCHEME = /^[a-z][a-z0-9+.-]*:(?!\d)/i;
const LINE_FRAGMENT = /#L(\d+)/;
const WINDOWS_LINE_FRAGMENT = /#L(\d+)(?:-L\d+)?$/;
const LINE_NUMBER = /:(\d+)(:\d+)*$/;
const FENCE = /^\s*(```|~~~)/;
const PREVIEW_LINES = 5;

export type MarkdownSource =
  | { kind: "file"; path: string; base?: string }
  | { kind: "code"; path: string; line?: number; base?: string }
  | { kind: "inline"; title: string; text: string };

export interface DocumentSummary {
  title: string | null;
  lines: string[];
}

export interface Frontmatter {
  entries: [string, string][];
  body: string;
}

const safeDecode = (value: string, decode = decodeURI) => {
  try {
    return decode(value);
  } catch {
    return value;
  }
};

export const isWindowsPathLink = (url: string) => isWindowsDrivePath(safeDecode(url));

export const windowsPathMentionHref = (path: string) =>
  isWindowsDrivePath(path) ? path.replaceAll("%", "%25").replaceAll("#", "%23") : null;

export interface FileReference {
  path: string;
  line?: number;
}

export function fileReference(reference: string | undefined): FileReference | null {
  if (!reference) return null;
  let path = reference.trim().replace(/^<|>$/g, "");
  const windows = isWindowsPathLink(path);
  if (path.startsWith("file://")) path = path.slice("file://".length);
  else if (path.startsWith("#") || (!windows && URL_SCHEME.test(path))) return null;
  const fragment = (windows ? WINDOWS_LINE_FRAGMENT : LINE_FRAGMENT).exec(path);
  path = windows
    ? safeDecode(path.slice(0, fragment?.index).split("?")[0], decodeURIComponent)
    : safeDecode(path.split(/[?#]/)[0]);
  const suffixLine = LINE_NUMBER.exec(path)?.[1];
  path = path.replace(LINE_NUMBER, "");
  if (!path) return null;
  const line = Number(fragment?.[1] ?? suffixLine);
  return line > 0 ? { path, line } : { path };
}

export function markdownPath(reference: string | undefined): string | null {
  const path = fileReference(reference)?.path;
  return path && MARKDOWN_PATH.test(path) ? path : null;
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
  const code = [...prose.matchAll(/`([^`\s]+)`/g)].map(
    ([, value]) => windowsPathMentionHref(value) ?? value
  );
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
  if (isWindowsDrivePath(path)) {
    const folder = folders.find((candidate) => isWithinPath(path, candidate));
    if (!folder) return path;
    const root = folder.replace(/[\\/]+$/, "");
    return path.slice(root.length + 1).replaceAll("\\", "/") || path;
  }
  const folder = folders.find((candidate) => path.startsWith(`${candidate}/`));
  return folder ? path.slice(folder.length + 1) : path;
}

const withLine = (path: string, line?: number) => (line ? `${path}:${line}` : path);

const namesPath = (text: string, path: string) => {
  const written = text.trim().replace(LINE_NUMBER, "");
  if (isWindowsDrivePath(path) && written) {
    const target = path.replaceAll("\\", "/").toLowerCase();
    const label = written.replaceAll("\\", "/").toLowerCase();
    return target === label || target.endsWith(`/${label}`);
  }
  return written !== "" && (path === written || path.endsWith(`/${written}`));
};

export const fileLinkLabel = ({ path, line }: FileReference, folders: string[], text: string) =>
  text.trim() && !namesPath(text, path) ? text : withLine(relativeTo(path, folders), line);

export interface PathCopy {
  label: string;
  value: string;
}

export function fileLinkCopies({ path, line }: FileReference, folders: string[]): PathCopy[] {
  const relative = relativeTo(path, folders);
  const asWritten = { label: "Copy path", value: withLine(path, line) };
  if (relative === path) return [asWritten];
  return [{ label: "Copy relative path", value: withLine(relative, line) }, asWritten];
}

export const isPlanItem = (item: ChatItem) =>
  item.kind === "notice" && item.title === "Plan" && item.text.trim() !== "";
