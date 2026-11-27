import { expect, it } from "vitest";
import { ChatItem, SubagentLaunch } from "../../services/chat";
import { launchesOf, withSubagentLaunches } from "./subagentLaunches";

const launch = (taskId: string, conversationId: string): SubagentLaunch => ({
  taskId,
  agent: "claude",
  tab: "a1",
  conversationId,
});

const command = (id: string, title: string, extra: Partial<ChatItem> = {}): ChatItem => ({
  id,
  kind: "command",
  text: "",
  title,
  ...extra,
});

const placed = (items: ChatItem[]) =>
  items.map((item) => [item.id, launchesOf(item).map((each) => each.conversationId)]);

it("leaves the items alone when the chat launched no subagent", () => {
  const items = [command("c1", "ls")];
  expect(withSubagentLaunches(items, [], true)).toBe(items);
});

it("gives each subagent of a chained launch its card on that command", () => {
  const chained = "wtm task chat t1 --subagent && wtm task chat t2 --subagent";
  const printed = launch("t2", "b");
  const items = [command("c1", chained, { subagentLaunch: printed })];
  const named = { ...printed, name: "Review" };
  const shown = withSubagentLaunches(items, [launch("t1", "a"), named], true);
  expect(placed(shown)).toEqual([["c1", ["a", "b"]]]);
  expect(shown[0].subagentLaunches?.[1]).toEqual(named);
});

it("places a launch whose output was not read on the command naming its task", () => {
  const items = [
    command("c1", "wtm task chat t1 --subagent &"),
    command("c2", "wtm task chat t2 --subagent > /dev/null"),
    command("c3", "wtm task get t1"),
  ];
  const shown = withSubagentLaunches(items, [launch("t1", "a"), launch("t2", "b")], true);
  expect(placed(shown)).toEqual([
    ["c1", ["a"]],
    ["c2", ["b"]],
    ["c3", []],
  ]);
});

it("places repeated launches into one task on its latest commands", () => {
  const items = [
    command("c1", "wtm task chat t1 --subagent"),
    command("c2", "wtm task chat t1 --subagent"),
  ];
  const once = withSubagentLaunches(items, [launch("t1", "b")], true);
  expect(placed(once)).toEqual([
    ["c1", []],
    ["c2", ["b"]],
  ]);
  const thrice = withSubagentLaunches(
    items,
    [launch("t1", "a"), launch("t1", "b"), launch("t1", "c")],
    true
  );
  expect(placed(thrice)).toEqual([
    ["c1", ["a", "b"]],
    ["c2", ["c"]],
  ]);
});

it("places a launch no command names on the last launch command, or after the chat", () => {
  const items = [command("c1", 'for t in $TASKS; do wtm task chat "$t" --subagent; done')];
  expect(placed(withSubagentLaunches(items, [launch("t1", "a")], true))).toEqual([["c1", ["a"]]]);
  const scripted = withSubagentLaunches([command("c1", "./launch.sh")], [launch("t1", "a")], true);
  expect(placed(scripted)).toEqual([
    ["c1", []],
    ["subagent:a", ["a"]],
  ]);
});

it("waits for earlier items before placing a launch no loaded command names", () => {
  const items = [command("c1", "wtm task chat t2 --subagent"), command("c2", "./launch.sh")];
  const shown = withSubagentLaunches(items, [launch("t1", "a"), launch("t2", "b")], false);
  expect(placed(shown)).toEqual([
    ["c1", ["b"]],
    ["c2", []],
  ]);
});

it("keeps the subagents commands printed before the chat recorded them", () => {
  const items = [
    command("c1", "wtm task chat t1 --subagent", { subagentLaunch: launch("t1", "a") }),
  ];
  expect(placed(withSubagentLaunches(items, [launch("t2", "b")], false))).toEqual([["c1", ["a"]]]);
});
