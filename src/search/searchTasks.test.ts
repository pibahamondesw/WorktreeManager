import { describe, it, expect } from "vitest";
import { searchTasks } from "./searchTasks";
import { Task, TaskMember, Workspace } from "../types";

const workspaces: Workspace[] = [
  { id: "ws-web", name: "Web", repos: [] },
  { id: "ws-api", name: "API", repos: [] },
];

function member(overrides: Partial<TaskMember> = {}): TaskMember {
  return {
    repoId: "r1",
    repoName: "frontend",
    localPath: "/repos/frontend",
    path: "/worktrees/frontend/branch",
    branchName: "feature/branch",
    ...overrides,
  };
}

function task(overrides: Partial<Task> = {}): Task {
  return {
    id: "t1",
    workspaceId: "ws-web",
    branchName: "feature/branch",
    members: [member()],
    createdAt: "2026-01-01T00:00:00.000Z",
    ...overrides,
  };
}

const quickSearch = task({
  id: "quick-search",
  linearIssueIdentifier: "WOR-49",
  linearIssueTitle: "Quick search de tarea",
  branchName: "pedrobahamondes/wor-49-quick-search-de-tarea",
  createdAt: "2026-02-01T00:00:00.000Z",
});

const apiFix = task({
  id: "api-fix",
  workspaceId: "ws-api",
  linearIssueIdentifier: "WOR-12",
  linearIssueTitle: "Fix search timeout",
  branchName: "pedrobahamondes/wor-12-fix-timeout",
  members: [member({ repoName: "backend", path: "/worktrees/backend/wor-12" })],
  createdAt: "2026-03-01T00:00:00.000Z",
});

function run(query: string, selectedWorkspaceId: string | null = "ws-web") {
  return searchTasks({
    tasks: [quickSearch, apiFix],
    workspaces,
    selectedWorkspaceId,
    query,
  });
}

const ids = (query: string, selectedWorkspaceId?: string | null) =>
  run(query, selectedWorkspaceId).map((r) => r.task.id);

describe("searchTasks", () => {
  it("returns every task for a blank query, current workspace first", () => {
    expect(ids("")).toEqual(["quick-search", "api-fix"]);
    expect(ids("", "ws-api")).toEqual(["api-fix", "quick-search"]);
  });

  it("matches the Linear identifier case-insensitively", () => {
    expect(ids("wor-49")).toEqual(["quick-search"]);
  });

  it("matches title text and branch text", () => {
    expect(ids("timeout")).toEqual(["api-fix"]);
    expect(ids("de-tarea")).toEqual(["quick-search"]);
  });

  it("puts current-workspace tasks ahead of better text matches elsewhere", () => {
    // "search" hits api-fix's title word-start and quick-search's title word-start,
    // but quick-search is in the selected workspace.
    expect(ids("search")).toEqual(["quick-search", "api-fix"]);
    expect(ids("search", "ws-api")).toEqual(["api-fix", "quick-search"]);
  });

  it("ranks an exact identifier above a title hit within the same workspace", () => {
    const results = searchTasks({
      tasks: [
        task({ id: "title-hit", linearIssueTitle: "wor-49 mentioned here" }),
        task({ id: "id-hit", linearIssueIdentifier: "WOR-49" }),
      ],
      workspaces,
      selectedWorkspaceId: "ws-web",
      query: "wor-49",
    });
    expect(results.map((r) => r.task.id)).toEqual(["id-hit", "title-hit"]);
  });

  it("ANDs multiple terms", () => {
    expect(ids("fix timeout")).toEqual(["api-fix"]);
    expect(ids("fix de-tarea")).toEqual([]);
  });

  it("scopes with in: and supports OR values", () => {
    expect(ids("in:api")).toEqual(["api-fix"]);
    expect(ids("in:web|api")).toEqual(["quick-search", "api-fix"]);
  });

  it("filters by repo, branch, id and path", () => {
    expect(ids("repo:backend")).toEqual(["api-fix"]);
    expect(ids("branch:wor-12")).toEqual(["api-fix"]);
    expect(ids("id:wor-49")).toEqual(["quick-search"]);
    expect(ids("path:worktrees/backend")).toEqual(["api-fix"]);
  });

  it("excludes with negated terms and filters", () => {
    expect(ids("-fix")).toEqual(["quick-search"]);
    expect(ids("-in:web")).toEqual(["api-fix"]);
    expect(ids("search -in:api")).toEqual(["quick-search"]);
  });

  it("breaks ties by newest first", () => {
    const older = task({ id: "older", createdAt: "2026-01-01T00:00:00.000Z" });
    const newer = task({ id: "newer", createdAt: "2026-05-01T00:00:00.000Z" });
    const results = searchTasks({
      tasks: [older, newer],
      workspaces,
      selectedWorkspaceId: "ws-web",
      query: "",
    });
    expect(results.map((r) => r.task.id)).toEqual(["newer", "older"]);
  });

  it("attaches the workspace and current-workspace flag to each result", () => {
    const [first] = run("wor-12");
    expect(first.workspace?.name).toBe("API");
    expect(first.inCurrentWorkspace).toBe(false);
  });
});
