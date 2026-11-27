import { ChatItem } from "../../services/chat";
import { ChatEntry } from "./chatEntries";
import { isPlanItem } from "./markdownDocument";
import { launchesOf } from "./subagentLaunches";

export type ToolVisibility = "all" | "hideSuccessful" | "hideAll" | "zen";

export const TOOL_VISIBILITY_OPTIONS: { id: ToolVisibility; label: string }[] = [
  { id: "all", label: "Show all tool calls" },
  { id: "hideSuccessful", label: "Hide successful tool calls" },
  { id: "hideAll", label: "Hide all tool calls" },
  { id: "zen", label: "Zen: prompts and final replies" },
];

const ALWAYS_SHOWN = new Set<ChatItem["kind"]>(["user", "question", "error", "handoff"]);
const REPLY_KINDS = new Set<ChatItem["kind"]>(["assistant", "review"]);

export const isFailedStep = (item: ChatItem) =>
  item.status === "failed" || item.status === "declined";

const isRunningOrFailed = (item: ChatItem) => item.status === "inProgress" || isFailedStep(item);

const startsTurn = (row: ChatEntry) =>
  row.kind === "item" && row.item.kind === "user" && row.item.status !== "queued";

/** The last reply before each turn's end; the running turn's appears once it ends. */
function finalReplies(rows: readonly ChatEntry[], turnRunning: boolean) {
  const finals = new Set<string>();
  let lastReply: string | null = null;
  for (const row of rows) {
    if (startsTurn(row)) {
      if (lastReply) finals.add(lastReply);
      lastReply = null;
    } else if (row.kind === "item" && REPLY_KINDS.has(row.item.kind)) lastReply = row.id;
  }
  if (lastReply && !turnRunning) finals.add(lastReply);
  return finals;
}

function shownRow(
  row: ChatEntry,
  visibility: ToolVisibility,
  finals: ReadonlySet<string>
): ChatEntry | null {
  if (row.kind === "work") {
    if (visibility !== "hideSuccessful") return null;
    const items = row.items.filter(isRunningOrFailed);
    if (items.length === 0) return null;
    return items.length === row.items.length ? row : { ...row, items };
  }
  const { item } = row;
  if (ALWAYS_SHOWN.has(item.kind) || launchesOf(item).length > 0) return row;
  if (item.kind === "agent")
    return visibility === "hideSuccessful" && isRunningOrFailed(item) ? row : null;
  if (visibility !== "zen") return row;
  return finals.has(row.id) || isPlanItem(item) ? row : null;
}

/**
 * The rows the chat shows under `visibility`. A hidden row's end passes to the row shown before
 * it, so a turn's change summary stays after the last thing shown.
 */
export function visibleRows(
  rows: ChatEntry[],
  visibility: ToolVisibility,
  turnRunning: boolean
): ChatEntry[] {
  if (visibility === "all") return rows;
  const finals = visibility === "zen" ? finalReplies(rows, turnRunning) : new Set<string>();
  const shown: ChatEntry[] = [];
  for (const row of rows) {
    const visible = shownRow(row, visibility, finals);
    if (visible) shown.push(visible);
    else if (shown.length > 0) shown.push({ ...shown.pop()!, lastIndex: row.lastIndex });
  }
  return shown;
}
