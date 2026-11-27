import { ChatItem, SubagentLaunch } from "../../services/chat";

/** The subagents a command shows: those placed on it, or the one its output names. */
export const launchesOf = (item: ChatItem): SubagentLaunch[] =>
  item.subagentLaunches ?? (item.subagentLaunch ? [item.subagentLaunch] : []);

const isLaunchCommand = (item: ChatItem) =>
  item.kind === "command" && !item.parent && !!item.title?.includes("--subagent");

const launchItem = (launch: SubagentLaunch): ChatItem => ({
  id: `subagent:${launch.conversationId}`,
  kind: "command",
  text: "",
  subagentLaunches: [launch],
});

/**
 * Places each subagent the chat launched, in launch order, on the command that launched it: the
 * one whose output names it, or else one naming its task, the latest launches on the latest
 * commands. A launch no command names goes on the last launch command, or after the
 * conversation. While earlier items are not loaded, such a launch may be among them, so it waits.
 */
export function withSubagentLaunches(
  items: ChatItem[],
  launched: SubagentLaunch[],
  complete: boolean
): ChatItem[] {
  if (launched.length === 0) return items;
  const placed = new Map<string, SubagentLaunch[]>();
  const place = (item: ChatItem, launch: SubagentLaunch) =>
    placed.set(item.id, [...(placed.get(item.id) ?? []), launch]);
  const registered = new Set(launched.map((launch) => launch.conversationId));
  const printedBy = new Map<string, ChatItem>();
  for (const item of items) {
    if (!item.subagentLaunch) continue;
    printedBy.set(item.subagentLaunch.conversationId, item);
    if (!registered.has(item.subagentLaunch.conversationId)) place(item, item.subagentLaunch);
  }
  const commands = items.filter(isLaunchCommand);
  const unprinted = launched.filter((launch) => !printedBy.has(launch.conversationId));
  const orphans: SubagentLaunch[] = [];
  for (const launch of launched) {
    const printer = printedBy.get(launch.conversationId);
    if (printer) {
      place(printer, launch);
      continue;
    }
    const ofTask = unprinted.filter((other) => other.taskId === launch.taskId);
    const naming = commands.filter(
      (command) =>
        command.title?.includes(launch.taskId) && command.subagentLaunch?.taskId !== launch.taskId
    );
    const latestFirst = naming.length - ofTask.length + ofTask.indexOf(launch);
    const command = naming[latestFirst] ?? (complete ? (naming[0] ?? commands.at(-1)) : undefined);
    if (command) place(command, launch);
    else if (complete) orphans.push(launch);
  }
  const shown = items.map((item) => {
    const launches = placed.get(item.id);
    return launches ? { ...item, subagentLaunches: launches } : item;
  });
  return [...shown, ...orphans.map(launchItem)];
}
