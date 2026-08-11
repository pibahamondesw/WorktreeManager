import { describe, it, expect } from "vitest";
import { buildTaskNote, taskNoteFileName, taskNotePath, taskNoteUri } from "./notes";
import { Task, TaskMember, Workspace } from "../types";

function member(repoName: string, path: string): TaskMember {
  return { repoId: `${repoName}-id`, repoName, localPath: `/repos/${repoName}`, path, branchName: "b" };
}

function task(overrides: Partial<Task> = {}): Task {
  return {
    id: "t1",
    workspaceId: "w1",
    branchName: "pedro/wor-39-evaluar-obsidian",
    linearIssueIdentifier: "WOR-39",
    linearIssueTitle: "Evaluar uso de Obsidian",
    members: [member("worktreemanager", "/wt/worktreemanager/pedro/wor-39-evaluar-obsidian")],
    createdAt: "2026-08-04T00:00:00.000Z",
    ...overrides,
  };
}

const workspace: Workspace = {
  id: "w1",
  name: "WorktreeManager",
  repos: [],
  notesPath: "/vault/task-logs",
};

describe("taskNoteFileName", () => {
  it("prefixes with the issue ID and drops it from the slug", () => {
    expect(taskNoteFileName(task())).toBe("WOR-39-evaluar-obsidian.md");
  });

  it("falls back to the branch slug with no Linear issue", () => {
    const t = task({
      linearIssueIdentifier: undefined,
      linearIssueTitle: undefined,
      branchName: "fix/flaky-webhook-spec",
    });
    expect(taskNoteFileName(t)).toBe("flaky-webhook-spec.md");
  });

  it("uses the bare issue ID when the branch carries nothing else", () => {
    expect(taskNoteFileName(task({ branchName: "pedro/wor-39" }))).toBe("WOR-39.md");
  });

  it("uppercases a lowercase issue identifier", () => {
    expect(taskNoteFileName(task({ linearIssueIdentifier: "wor-39" }))).toBe(
      "WOR-39-evaluar-obsidian.md"
    );
  });
});

describe("buildTaskNote", () => {
  it("renders frontmatter with the issue, branch, and workspace", () => {
    const { contents } = buildTaskNote(task(), workspace);
    expect(contents).toContain('title: "WOR-39 — Evaluar uso de Obsidian"');
    expect(contents).toContain("type: task-log");
    expect(contents).toContain("status: active");
    expect(contents).toContain("tickets: [WOR-39]");
    expect(contents).toContain('branch: "pedro/wor-39-evaluar-obsidian"');
    expect(contents).toContain('workspace: "WorktreeManager"');
    expect(contents).toContain("repos: [worktreemanager]");
  });

  it("lists one worktrees entry per member", () => {
    const t = task({
      members: [member("api", "/wt/api/branch"), member("web", "/wt/web/branch")],
    });
    const { contents } = buildTaskNote(t, workspace);
    expect(contents).toContain("repos: [api, web]");
    expect(contents).toContain('  - repo: api\n    path: "/wt/api/branch"');
    expect(contents).toContain('  - repo: web\n    path: "/wt/web/branch"');
  });

  it("emits an empty worktrees list when a task has no members", () => {
    const { contents } = buildTaskNote(task({ members: [] }), workspace);
    expect(contents).toContain("worktrees: []");
    expect(contents).toContain("repos: []");
  });

  it("titles and tickets fall back cleanly with no Linear issue", () => {
    const t = task({ linearIssueIdentifier: undefined, linearIssueTitle: undefined });
    const { contents } = buildTaskNote(t, workspace);
    expect(contents).toContain('title: "pedro/wor-39-evaluar-obsidian"');
    expect(contents).toContain("tickets: []");
  });

  it("escapes double quotes in a title", () => {
    const { contents } = buildTaskNote(task({ linearIssueTitle: 'Fix "weird" bug' }), workspace);
    expect(contents).toContain('title: "WOR-39 — Fix \\"weird\\" bug"');
  });

  it("includes the four body sections", () => {
    const { contents } = buildTaskNote(task(), workspace);
    for (const heading of ["## Context", "## Decisions", "## Learnings", "## Log"]) {
      expect(contents).toContain(heading);
    }
  });
});

describe("taskNotePath", () => {
  it("joins the notes folder and file name", () => {
    expect(taskNotePath(workspace, task())).toBe("/vault/task-logs/WOR-39-evaluar-obsidian.md");
  });

  it("tolerates a trailing slash on the notes folder", () => {
    expect(taskNotePath({ ...workspace, notesPath: "/vault/task-logs/" }, task())).toBe(
      "/vault/task-logs/WOR-39-evaluar-obsidian.md"
    );
  });

  it("returns null when notes are not configured", () => {
    expect(taskNotePath({ ...workspace, notesPath: null }, task())).toBeNull();
    expect(taskNotePath({ ...workspace, notesPath: "  " }, task())).toBeNull();
  });
});

describe("taskNoteUri", () => {
  it("percent-encodes the path", () => {
    expect(taskNoteUri("/vault/task logs/WOR-39.md")).toBe(
      "obsidian://open?path=%2Fvault%2Ftask%20logs%2FWOR-39.md"
    );
  });
});
