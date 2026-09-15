import { describe, expect, it } from "vitest";
import { buildCommands } from "./commands";
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
    {
      repoId: "r2",
      repoName: "api",
      localPath: "/repos/api",
      path: "/wt/api",
      branchName: "feat/palette",
    },
  ],
  createdAt: "2026-01-01T00:00:00Z",
};

function ids(overrides: Partial<Parameters<typeof buildCommands>[0]> = {}) {
  return buildCommands({
    workspaces,
    selectedWorkspaceId: "ws-1",
    tasks: [task],
    themeId: "default",
    editorApp: "cursor",
    ...overrides,
  }).map((c) => c.id);
}

describe("buildCommands", () => {
  it("exposes switch/edit/remove for each workspace and settings", () => {
    const all = ids();
    expect(all).toEqual(
      expect.arrayContaining([
        "switch-ws:ws-1",
        "switch-ws:ws-2",
        "edit-ws:ws-1",
        "remove-ws:ws-1",
        "new-task",
        "vault",
        "theme-picker",
        "theme:default",
        "editor:cursor",
        "editor:claude-code",
      ])
    );
  });

  it("only empty-shows switch for other workspaces and edit/remove for the current one", () => {
    const commands = buildCommands({
      workspaces,
      selectedWorkspaceId: "ws-1",
      tasks: [task],
      themeId: "default",
      editorApp: "cursor",
    });
    const empty = commands.filter((c) => c.emptyVisible).map((c) => c.id);
    expect(empty).toContain("switch-ws:ws-2");
    expect(empty).not.toContain("switch-ws:ws-1");
    expect(empty).toContain("edit-ws:ws-1");
    expect(empty).not.toContain("edit-ws:ws-2");
    expect(empty).toContain("remove-ws:ws-1");
    expect(empty).toContain("new-task");
    expect(empty).toContain("vault");
    expect(empty).toContain("theme-picker");
    expect(empty).not.toContain("theme:default");
  });

  it("omits New task when no workspace is selected", () => {
    expect(ids({ selectedWorkspaceId: null })).not.toContain("new-task");
  });

  it("emits a PR command per member repo plus copy-linear and Claude", () => {
    const all = ids();
    expect(all).toEqual(
      expect.arrayContaining([
        "copy-linear:t-1",
        "open-pr:t-1:r1",
        "open-pr:t-1:r2",
        "open-claude:t-1",
      ])
    );
    expect(all.filter((id) => id.startsWith("open-pr:t-1:")).length).toBe(2);
  });

  it("includes existing shortcuts for runnable commands", () => {
    const commands = buildCommands({
      workspaces,
      selectedWorkspaceId: "ws-1",
      tasks: [task],
      themeId: "default",
      editorApp: "cursor",
    });

    expect(commands.find((command) => command.id === "new-task")?.shortcut).toEqual({
      key: "n",
      label: "N",
    });
    expect(commands.find((command) => command.id === "switch-ws:ws-2")?.shortcut).toEqual({
      key: "1",
      label: "⌘1",
      meta: true,
    });
  });
});
