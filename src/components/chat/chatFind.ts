import { ChatItem } from "../../services/chat";
import { ChatEntry } from "./chatEntries";
import { isPlanItem } from "./markdownDocument";

export interface FindMatch {
  rowKey: string;
  occurrence: number;
}

const SEARCHABLE_KINDS = new Set<ChatItem["kind"]>([
  "user",
  "assistant",
  "review",
  "error",
  "notice",
  "handoff",
]);

const isSearchable = (item: ChatItem) => SEARCHABLE_KINDS.has(item.kind) && !isPlanItem(item);

const escapeRegExp = (text: string) => text.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");

/** Where `query` appears in `text`, ignoring case, as offsets into `text` itself. */
function occurrences(text: string, query: string) {
  if (!query) return [];
  return Array.from(text.matchAll(new RegExp(escapeRegExp(query), "giu")), (match) => ({
    start: match.index,
    end: match.index + match[0].length,
  }));
}

/** Every occurrence of `query` in the rows' messages, oldest first, ignoring case. */
export function findMatches(rows: readonly ChatEntry[], query: string): FindMatch[] {
  return rows.flatMap((row) => {
    if (row.kind !== "item" || !isSearchable(row.item)) return [];
    return occurrences(row.item.text, query).map((_, occurrence) => ({
      rowKey: row.id,
      occurrence,
    }));
  });
}

export const sameMatch = (a: FindMatch | null, b: FindMatch | null) =>
  a?.rowKey === b?.rowKey && a?.occurrence === b?.occurrence;

const skipsText = (node: Node) =>
  !!node.parentElement?.closest("button, [aria-hidden=true], .sr-only");

/** The ranges where `query` appears in the text `root` renders, ignoring case and controls. */
export function textRanges(root: Node, query: string): Range[] {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (node) => (skipsText(node) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
  });
  const nodes: { node: Text; start: number }[] = [];
  let text = "";
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    nodes.push({ node: node as Text, start: text.length });
    text += node.textContent ?? "";
  }
  const positionAt = (offset: number) => {
    let index = nodes.length - 1;
    while (index > 0 && nodes[index].start > offset) index--;
    return { node: nodes[index].node, offset: offset - nodes[index].start };
  };
  return occurrences(text, query).map(({ start, end }) => {
    const from = positionAt(start);
    const to = positionAt(end);
    const range = document.createRange();
    range.setStart(from.node, from.offset);
    range.setEnd(to.node, to.offset);
    return range;
  });
}
