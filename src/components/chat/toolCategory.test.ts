import { expect, it } from "vitest";
import { ChatItem } from "../../services/chat";
import { summarizeWork, toolCategory } from "./toolCategory";

const command = (id: string, commandActivity?: ChatItem["commandActivity"]): ChatItem => ({
  id,
  kind: "command",
  text: "",
  title: "ls",
  commandActivity,
});
const edit = (id: string): ChatItem => ({ id, kind: "fileChange", text: "", title: "Edit a.ts" });
const tool = (id: string, title?: string): ChatItem => ({ id, kind: "tool", text: "", title });

it.each([
  [command("c"), "command"],
  [command("r", { kind: "read", paths: ["/a.ts"] }), "read"],
  [command("s", { kind: "search" }), "search"],
  [edit("e"), "edit"],
  [tool("r", "Read"), "read"],
  [tool("g", "Grep"), "search"],
  [tool("w", "Web search"), "web"],
  [tool("m", "github · create_issue"), "other"],
  [tool("p", "constructor"), "other"],
  [tool("u"), "other"],
] as const)("classifies %o as %s", (item, category) => {
  expect(toolCategory(item)).toBe(category);
});

it("names edits and commands before reads and counts the rest", () => {
  const steps = [tool("r1", "Read"), command("c1"), tool("r2", "Read"), edit("e1"), tool("x")];

  expect(summarizeWork(steps)).toBe("Ran 1 command · Made 1 edit · +3");
});

it("falls back to reads and searches in the order they first appeared", () => {
  const steps = [tool("g", "Glob"), tool("r1", "Read"), tool("r2", "Read"), tool("x")];

  expect(summarizeWork(steps)).toBe("Searched once · Read 2 files · +1");
});

it("omits the remainder when every step is named", () => {
  expect(summarizeWork([tool("f", "WebFetch"), tool("x"), tool("y")])).toBe(
    "Browsed the web once · Used 2 tools"
  );
});

it.each([false, true])("counts unique read paths with details loaded = %s", (expanded) => {
  const steps = [
    { ...tool("r1", "Read"), readPath: "/a.ts", detail: expanded ? "/a.ts" : undefined },
    { ...tool("r2", "Read"), readPath: "/a.ts", detail: expanded ? "File changed" : undefined },
    { ...tool("r3", "NotebookRead"), readPath: "/b.ipynb" },
  ];

  expect(summarizeWork(steps)).toBe("Read 2 files");
});

it("counts each read with an unknown path separately even when details match", () => {
  const steps = [
    { ...tool("r1", "Read"), detail: "Missing path" },
    { ...tool("r2", "Read"), detail: "Missing path" },
  ];

  expect(summarizeWork(steps)).toBe("Read 2 files");
});

it("summarizes Codex exploration like Claude and counts distinct paths without command output", () => {
  const steps = [
    command("r1", { kind: "read", paths: ["/a.ts", "/b.ts"] }),
    { ...command("r2", { kind: "read", paths: ["/a.ts"] }), detail: "file contents" },
    { ...tool("r3", "Read"), readPath: "/b.ts" },
    command("s", { kind: "search" }),
  ];

  expect(summarizeWork(steps)).toBe("Read 2 files · Searched once");
});

it("counts repeated web and code searches", () => {
  const steps = [
    tool("g1", "Grep"),
    tool("g2", "Grep"),
    tool("w1", "WebSearch"),
    tool("w2", "WebFetch"),
  ];

  expect(summarizeWork(steps)).toBe("Searched 2 times · Browsed the web 2 times");
});
