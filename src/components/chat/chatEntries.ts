import { ChatItem } from "../../services/chat";

const WORK_KINDS = new Set<ChatItem["kind"]>(["reasoning", "command", "fileChange", "tool"]);

export type ChatEntry =
  | { kind: "item"; id: string; item: ChatItem; lastIndex: number }
  | { kind: "work"; id: string; items: ChatItem[]; lastIndex: number };

export type SubagentSteps = ReadonlyMap<string, ChatItem[]>;

const isWorkItem = (item: ChatItem) => WORK_KINDS.has(item.kind);

export const isMainItem = (item: ChatItem) => !item.parent;

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

/** Each subagent run's steps in arrival order, keyed by the run's `agent` item id. */
export function subagentSteps(items: ChatItem[]): SubagentSteps {
  const steps = new Map<string, ChatItem[]>();
  for (const item of items) {
    if (!item.parent) continue;
    const run = steps.get(item.parent);
    if (run) run.push(item);
    else steps.set(item.parent, [item]);
  }
  return steps;
}
