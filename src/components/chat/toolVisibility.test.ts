import { expect, it } from "vitest";
import { ChatItem } from "../../services/chat";
import { chatEntries } from "./chatEntries";
import { visibleRows } from "./toolVisibility";

const item = (id: string, kind: ChatItem["kind"], extra: Partial<ChatItem> = {}): ChatItem => ({
  id,
  kind,
  text: id,
  ...extra,
});

const conversation = [
  item("u1", "user"),
  item("think", "reasoning", { text: "Looking around" }),
  item("read", "tool", { title: "Read", status: "completed" }),
  item("build", "command", { status: "failed" }),
  item("test", "command", { status: "inProgress" }),
  item("note", "assistant"),
  item("sub", "agent", { status: "completed" }),
  item("sub-failed", "agent", { status: "failed" }),
  item("ask", "question", { status: "completed" }),
  item("plan", "notice", { title: "Plan" }),
  item("compacted", "notice"),
  item("oops", "error"),
  item("final", "assistant"),
  item("u2", "user", { status: "steered" }),
  item("edit", "fileChange", { status: "completed" }),
  item("reply", "assistant"),
  item("u3", "user", { status: "queued" }),
];

const rowIds = (rows: ReturnType<typeof visibleRows>) =>
  rows.map((row) => (row.kind === "item" ? row.id : row.items.map((step) => step.id).join("+")));

it("shows every row unchanged by default", () => {
  const rows = chatEntries(conversation);
  expect(visibleRows(rows, "all", false)).toBe(rows);
});

it("keeps only running and failed calls when hiding successful ones", () => {
  expect(rowIds(visibleRows(chatEntries(conversation), "hideSuccessful", false))).toEqual([
    "u1",
    "think",
    "build+test",
    "note",
    "sub-failed",
    "ask",
    "plan",
    "compacted",
    "oops",
    "final",
    "u2",
    "reply",
    "u3",
  ]);
});

it("keeps a work row as is when none of its calls is hidden", () => {
  const rows = chatEntries([item("u", "user"), item("c", "command", { status: "failed" })]);
  expect(visibleRows(rows, "hideSuccessful", false)[1]).toBe(rows[1]);
});

it("hides every call, subagents included, when hiding all", () => {
  expect(rowIds(visibleRows(chatEntries(conversation), "hideAll", false))).toEqual([
    "u1",
    "think",
    "note",
    "ask",
    "plan",
    "compacted",
    "oops",
    "final",
    "u2",
    "reply",
    "u3",
  ]);
});

it("shows prompts, each turn's last reply, plans, questions and errors in zen", () => {
  expect(rowIds(visibleRows(chatEntries(conversation), "zen", false))).toEqual([
    "u1",
    "ask",
    "plan",
    "oops",
    "final",
    "u2",
    "reply",
    "u3",
  ]);
});

it("holds back the running turn's reply in zen until the turn ends", () => {
  const rows = chatEntries(conversation);
  expect(rowIds(visibleRows(rows, "zen", true))).toEqual([
    "u1",
    "ask",
    "plan",
    "oops",
    "final",
    "u2",
    "u3",
  ]);
});

it("passes a hidden row's end to the row shown before it", () => {
  const rows = chatEntries([
    item("u", "user"),
    item("a", "assistant"),
    item("c", "command", { status: "completed" }),
  ]);
  const [prompt, reply] = visibleRows(rows, "hideAll", false);
  expect(prompt.lastIndex).toBe(0);
  expect(reply).toMatchObject({ id: "a", lastIndex: 2 });
});

it("drops hidden rows that come before anything shown", () => {
  const rows = chatEntries([item("c", "command", { status: "completed" }), item("u", "user")]);
  expect(rowIds(visibleRows(rows, "hideAll", false))).toEqual(["u"]);
});

it("keeps a subagent launch apart from other commands and always shows it", () => {
  const subagentLaunch = {
    taskId: "t2",
    agent: "codex" as const,
    tab: "a1",
    conversationId: "th1",
  };
  const rows = chatEntries([
    item("u", "user"),
    item("ls", "command", { status: "completed" }),
    item("launch", "command", { status: "completed", subagentLaunch }),
    item("pwd", "command", { status: "completed" }),
    item("reply", "assistant"),
  ]);
  expect(rows.map((row) => row.kind)).toEqual(["item", "work", "item", "work", "item"]);
  for (const visibility of ["hideSuccessful", "hideAll", "zen"] as const)
    expect(rowIds(visibleRows(rows, visibility, false))).toContain("launch");
});
