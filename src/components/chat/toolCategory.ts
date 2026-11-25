import { ChatItem } from "../../services/chat";

export type ToolCategory = "edit" | "command" | "read" | "search" | "web" | "other";

const CATEGORY_BY_TOOL = new Map<string, ToolCategory>([
  ["Read", "read"],
  ["NotebookRead", "read"],
  ["Grep", "search"],
  ["Glob", "search"],
  ["LS", "search"],
  ["WebFetch", "web"],
  ["WebSearch", "web"],
  ["Web search", "web"],
]);

const SUMMARY_PRIORITY: Record<ToolCategory, number> = {
  edit: 0,
  command: 0,
  read: 1,
  search: 1,
  web: 1,
  other: 2,
};

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;
const times = (count: number) => (count === 1 ? "once" : `${count} times`);

const SUMMARY_LABEL: Record<ToolCategory, (count: number) => string> = {
  edit: (count) => `Made ${plural(count, "edit")}`,
  command: (count) => `Ran ${plural(count, "command")}`,
  read: (count) => `Read ${plural(count, "file")}`,
  search: (count) => `Searched ${times(count)}`,
  web: (count) => `Browsed the web ${times(count)}`,
  other: (count) => `Used ${plural(count, "tool")}`,
};

const SUMMARIZED_CATEGORIES = 2;

export function toolCategory(item: ChatItem): ToolCategory {
  if (item.kind === "fileChange") return "edit";
  if (item.kind === "command") return "command";
  return CATEGORY_BY_TOOL.get(item.title ?? "") ?? "other";
}

/** Distinct files read; a read whose path isn't known counts on its own. */
const readFiles = (reads: ChatItem[]) =>
  new Set(reads.map((read) => read.detail?.split("\n", 1)[0] ?? read.id)).size;

/** Names the two most telling kinds of step, edits and commands first, and counts the rest. */
export function summarizeWork(steps: ChatItem[]): string {
  const byCategory = new Map<ToolCategory, ChatItem[]>();
  for (const step of steps) {
    const category = toolCategory(step);
    const group = byCategory.get(category);
    if (group) group.push(step);
    else byCategory.set(category, [step]);
  }
  const summarized = [...byCategory]
    .map(([category, items], firstSeen) => ({
      category,
      steps: items.length,
      count: category === "read" ? readFiles(items) : items.length,
      firstSeen,
    }))
    .sort(
      (a, b) =>
        SUMMARY_PRIORITY[a.category] - SUMMARY_PRIORITY[b.category] || a.firstSeen - b.firstSeen
    )
    .slice(0, SUMMARIZED_CATEGORIES)
    .sort((a, b) => a.firstSeen - b.firstSeen);
  const remaining = steps.length - summarized.reduce((total, group) => total + group.steps, 0);
  const labels = summarized.map(({ category, count }) => SUMMARY_LABEL[category](count));
  return remaining > 0 ? [...labels, `+${remaining}`].join(" · ") : labels.join(" · ");
}
