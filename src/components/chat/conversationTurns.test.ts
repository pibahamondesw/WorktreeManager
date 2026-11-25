import { describe, expect, it } from "vitest";
import { ChatItem } from "../../services/chat";
import { chatEntries } from "./chatEntries";
import { conversationTurns, turnAt } from "./conversationTurns";

const rows = chatEntries([
  { id: "intro", kind: "assistant", text: "Resumed" },
  { id: "u1", kind: "user", text: "Fix the bug" },
  { id: "c", kind: "command", text: "grep bug" },
  { id: "a1", kind: "assistant", text: "Looking" },
  { id: "a2", kind: "assistant", text: "Fixed it" },
  { id: "u2", kind: "user", text: "Thanks" },
  { id: "n", kind: "notice", text: "Compacted" },
] as ChatItem[]);

describe("conversationTurns", () => {
  it("pairs each user message with the last reply before the next one", () => {
    expect(conversationTurns(rows)).toEqual([
      { rowKey: "u1", rowIndex: 1, prompt: "Fix the bug", reply: "Fixed it" },
      { rowKey: "u2", rowIndex: 5, prompt: "Thanks", reply: null },
    ]);
  });

  it("finds the turn holding a row", () => {
    const turns = conversationTurns(rows);
    expect(turnAt(turns, 0)).toBe(-1);
    expect(turnAt(turns, 1)).toBe(0);
    expect(turnAt(turns, 4)).toBe(0);
    expect(turnAt(turns, 6)).toBe(1);
  });
});
