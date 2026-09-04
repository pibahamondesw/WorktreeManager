import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  buildTaskNote,
  taskNoteFileName,
  taskNotePath,
  taskNoteUri,
  ensureTaskNote,
  archiveTaskNote,
} from "./notes";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { Task, TaskMember, VaultConfig, Workspace } from "../types";

function member(repoName: string, path: string): TaskMember {
  return {
    repoId: `${repoName}-id`,
    repoName,
    localPath: `/repos/${repoName}`,
    path,
    branchName: "b",
  };
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
};

const vault: VaultConfig = { enabled: true, path: "/vault" };

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
  it("joins the vault's task-logs folder and file name", () => {
    expect(taskNotePath(vault, task())).toBe("/vault/task-logs/WOR-39-evaluar-obsidian.md");
  });

  it("tolerates a trailing slash on the vault path", () => {
    expect(taskNotePath({ ...vault, path: "/vault/" }, task())).toBe(
      "/vault/task-logs/WOR-39-evaluar-obsidian.md"
    );
  });

  it("returns null when the vault is off or unset", () => {
    expect(taskNotePath({ enabled: false, path: "/vault" }, task())).toBeNull();
    expect(taskNotePath({ enabled: true, path: "  " }, task())).toBeNull();
  });
});

describe("taskNoteUri", () => {
  it("percent-encodes the path", () => {
    expect(taskNoteUri("/vault/task logs/WOR-39.md")).toBe(
      "obsidian://open?path=%2Fvault%2Ftask%20logs%2FWOR-39.md"
    );
  });
});

describe("task note storage", () => {
  beforeEach(() => vi.clearAllMocks());

  it("isolates identical names by task folder", () => {
    const first = task({ noteFolder: "t1" });
    const second = task({ id: "t2", noteFolder: "t2" });
    expect(taskNotePath(vault, first)).toBe("/vault/task-logs/t1/WOR-39-evaluar-obsidian.md");
    expect(taskNotePath(vault, second)).toBe("/vault/task-logs/t2/WOR-39-evaluar-obsidian.md");
    expect(buildTaskNote(first, workspace).contents).toContain('task_id: "t1"');
  });

  it.each([undefined, "t1"])(
    "uses the same folder for creation and archive: %s",
    async (noteFolder) => {
      const current = task({ noteFolder });
      vi.mocked(invoke).mockResolvedValue("/resolved/note.md");
      expect(await ensureTaskNote(vault, workspace, current)).toBe("/resolved/note.md");
      await archiveTaskNote(vault, current);
      const location = {
        notesPath: "/vault/task-logs",
        fileName: taskNoteFileName(current),
        noteFolder: noteFolder ?? null,
      };
      expect(invoke).toHaveBeenNthCalledWith(
        1,
        "ensure_task_note",
        expect.objectContaining(location)
      );
      expect(invoke).toHaveBeenNthCalledWith(
        2,
        "archive_task_note",
        expect.objectContaining(location)
      );
    }
  );

  it("does not touch notes when the vault is disabled", async () => {
    const disabled = { ...vault, enabled: false };
    expect(await ensureTaskNote(disabled, workspace, task())).toBeNull();
    await archiveTaskNote(disabled, task());
    expect(invoke).not.toHaveBeenCalled();
  });
});
