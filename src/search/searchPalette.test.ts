import { describe, expect, it } from "vitest";
import { buildCommands } from "./commands";
import { searchPalette } from "./searchPalette";
import { Task, Workspace } from "../types";

const workspaces: Workspace[] = [
  { id: "ws-1", name: "Payments", repos: [] },
  { id: "ws-2", name: "Ledger", repos: [] },
];

const task: Task = {
  id: "t-1",
  workspaceId: "ws-1",
  branchName: "feat/palette",
  linearIssueIdentifier: "WOR-67",
  linearIssueTitle: "Command palette",
  members: [
    {
      repoId: "r1",
      repoName: "web",
      localPath: "/repos/web",
      path: "/wt/web",
      branchName: "feat/palette",
    },
  ],
  createdAt: "2026-01-01T00:00:00Z",
};

function run(query: string) {
  return searchPalette({
    tasks: [task],
    workspaces,
    selectedWorkspaceId: "ws-1",
    query,
    commands: buildCommands({
      workspaces,
      selectedWorkspaceId: "ws-1",
      tasks: [task],
      themeId: "default",
      editorApp: "cursor",
    }),
  });
}

const kinds = (query: string) => run(query).map((item) => item.kind);
const commandIds = (query: string) =>
  run(query)
    .filter((item) => item.kind === "command")
    .map((item) => item.id);
const taskIds = (query: string) =>
  run(query)
    .filter((item) => item.kind === "task")
    .map((item) => item.id);

describe("searchPalette", () => {
  it("keeps tasks first on a blank query so Enter still opens a task", () => {
    expect(kinds("")).toContain("task");
    expect(kinds("")[0]).toBe("task");
    expect(commandIds("")).toContain("switch-ws:ws-2");
    expect(commandIds("")).toContain("new-task");
    expect(commandIds("")).not.toContain("copy-linear:t-1");
  });

  it("lists matching task actions after the task for an identifier query", () => {
    expect(taskIds("wor-67")).toEqual(["t-1"]);
    expect(commandIds("wor-67")).toEqual(
      expect.arrayContaining(["copy-linear:t-1", "open-pr:t-1:r1", "open-claude:t-1"])
    );
    expect(kinds("wor-67")[0]).toBe("task");
  });

  it("finds New task by name", () => {
    expect(commandIds("new task")).toContain("new-task");
  });

  it("finds settings without a leading >", () => {
    expect(commandIds("theme")).toEqual(expect.arrayContaining(["theme-picker", "theme:default"]));
    expect(commandIds("codex")).toEqual(expect.arrayContaining(["editor:codex", "open-codex:t-1"]));
    expect(commandIds("claude")).toEqual(
      expect.arrayContaining(["editor:claude-code", "open-claude:t-1"])
    );
  });

  it("hides tasks when the query starts with >", () => {
    expect(taskIds(">")).toEqual([]);
    expect(commandIds(">")).toContain("copy-linear:t-1");
    expect(commandIds(">theme")).toContain("theme-picker");
    expect(commandIds(">theme")).not.toContain("switch-ws:ws-2");
  });
});

it("keeps project groups contiguous in ranked order and separates identical names by ID", () => {
  const tasks = [
    { ...task, id: "a", linearProjectId: "p1", linearProjectName: "API", createdAt: "2026-04-01" },
    { ...task, id: "b", linearProjectId: "p2", linearProjectName: "API", createdAt: "2026-03-01" },
    { ...task, id: "c", linearProjectId: "p1", linearProjectName: "API", createdAt: "2026-02-01" },
    { ...task, id: "d", createdAt: "2026-01-01" },
  ];
  const results = searchPalette({
    tasks,
    workspaces,
    selectedWorkspaceId: "ws-1",
    query: "",
    commands: [],
  });
  expect(results.map((item) => item.id)).toEqual(["a", "c", "b", "d"]);
});
