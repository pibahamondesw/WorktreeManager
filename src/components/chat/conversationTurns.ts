import { ChatEntry } from "./chatEntries";

export interface ConversationTurn {
  rowKey: string;
  rowIndex: number;
  prompt: string;
  reply: string | null;
}

/** Each user message in the rows with the last assistant reply before the next one. */
export function conversationTurns(rows: readonly ChatEntry[]): ConversationTurn[] {
  const turns: ConversationTurn[] = [];
  rows.forEach((row, rowIndex) => {
    if (row.kind !== "item") return;
    if (row.item.kind === "user")
      turns.push({ rowKey: row.id, rowIndex, prompt: row.item.text, reply: null });
    else if (row.item.kind === "assistant" && turns.length > 0) turns.at(-1)!.reply = row.item.text;
  });
  return turns;
}

/** The turn holding row `rowIndex`, or -1 when the row comes before the first user message. */
export function turnAt(turns: readonly ConversationTurn[], rowIndex: number) {
  let index = -1;
  while (index + 1 < turns.length && turns[index + 1].rowIndex <= rowIndex) index++;
  return index;
}
