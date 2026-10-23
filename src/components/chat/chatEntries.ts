import { ChatItem } from "../../services/chat";

const WORK_KINDS = new Set<ChatItem["kind"]>(["reasoning", "command", "fileChange", "tool"]);

export type ChatEntry =
  | { kind: "item"; id: string; item: ChatItem; lastIndex: number }
  | { kind: "work"; id: string; items: ChatItem[]; lastIndex: number };

const isWorkItem = (item: ChatItem) => WORK_KINDS.has(item.kind);

/** Collapses each run of consecutive work items (thinking, tools, edits) into one entry. */
export function chatEntries(items: ChatItem[]): ChatEntry[] {
  const entries: ChatEntry[] = [];
  items.forEach((item, index) => {
    const last = entries.at(-1);
    if (!isWorkItem(item)) entries.push({ kind: "item", id: item.id, item, lastIndex: index });
    else if (last?.kind === "work") {
      last.items.push(item);
      last.lastIndex = index;
    } else entries.push({ kind: "work", id: item.id, items: [item], lastIndex: index });
  });
  return entries;
}
