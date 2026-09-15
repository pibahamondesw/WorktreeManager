import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_STATE, AppState, Task, Workspace } from "../types";
import { Operations, CreateTaskInput } from "./operations";
import { archiveTaskNote } from "./notes";
import { closeTaskSessions } from "./taskSessions";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("./store", () => ({ persist: vi.fn() }));
vi.mock("./notes", () => ({
  ensureTaskNote: vi.fn().mockResolvedValue("/note"),
  archiveTaskNote: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("./taskSessions", () => ({ closeTaskSessions: vi.fn().mockResolvedValue(undefined) }));

const workspace: Workspace = {
  id: "w1",
  name: "Payments",
  repos: ["api", "web"].map((name) => ({
    id: name,
    name,
    localPath: `/repos/${name}`,
    worktreeBasePath: `/wt/${name}`,
  })),
};
const input: CreateTaskInput = {
  workspaceId: "w1",
  branchName: "pedro/wor-80",
  linearIssue: { id: "issue-id", identifier: "WOR-80", title: "Local CLI" },
};
const task: Task = {
  id: "t1",
  workspaceId: "w1",
  branchName: "feature",
  createdAt: "2026-09-11",
  members: workspace.repos.map((repo) => ({
    repoId: repo.id,
    repoName: repo.name,
    localPath: repo.localPath,
    path: `${repo.worktreeBasePath}/feature`,
    branchName: "feature",
  })),
};

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function fixture(tasks: Task[] = []) {
  let state: AppState = {
    ...DEFAULT_STATE,
    workspaces: [workspace],
    tasks,
    selectedWorkspaceId: workspace.id,
  };
  const save = vi.fn().mockResolvedValue(undefined);
  const publish = vi.fn((next: AppState) => {
    state = next;
  });
  const operations = new Operations(
    () => state,
    publish,
    () => "cursor",
    save
  );
  return { operations, save, publish };
}

async function native(command: string, args?: Record<string, unknown>) {
  if (command === "resolve_manual_worktree")
    return {
      branchName: `user/${args?.rawName}`,
      path: `${args?.worktreeBasePath}/user/${args?.rawName}`,
    };
  if (command === "git_worktree_add") return { warning: null };
  if (command === "doppler_setup") return { status: "skipped_no_config" };
  if (command === "install_node_deps") return { status: "installed" };
  if (command === "prepare_task_workspace") return "/generated/workspace.code-workspace";
  return undefined;
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(invoke).mockImplementation(native as typeof invoke);
  vi.mocked(closeTaskSessions).mockResolvedValue(undefined);
});

describe("shared operations", () => {
  it("validates every destination before creating and waits for setup", async () => {
    const { operations } = fixture();
    const setup = deferred();
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === "install_node_deps") {
        await setup.promise;
        return { status: "installed" };
      }
      return native(command, args);
    }) as typeof invoke);
    let complete = false;
    const creating = operations.createTask(input).then((result) => {
      complete = true;
      return result;
    });
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("install_node_deps", {
        worktreePath: "/wt/api/pedro/wor-80",
      })
    );
    expect(complete).toBe(false);
    expect(operations.getState().tasks).toHaveLength(1);
    setup.resolve();
    const result = await creating;
    expect(result.data.members).toHaveLength(2);
    expect(result.data.linearIssueId).toBe("issue-id");
    expect(result.data.workspaceFilePath).toBe("/generated/workspace.code-workspace");
    expect(result.setup).toHaveLength(2);
    expect(result.warnings).toEqual([]);
    const commands = vi.mocked(invoke).mock.calls.map(([command]) => command);
    expect(commands.indexOf("validate_task_worktrees")).toBeLessThan(
      commands.indexOf("git_worktree_add")
    );
    expect(commands).not.toContain("open_editor");
  });

  it("selects repositories and preserves manual branch resolution", async () => {
    const { operations } = fixture();
    const result = await operations.createTask({
      workspaceId: "w1",
      branchName: "feature",
      repoIds: ["web"],
    });
    expect(result.data.members).toEqual([
      {
        repoId: "web",
        repoName: "web",
        localPath: "/repos/web",
        path: "/wt/web/user/feature",
        branchName: "user/feature",
      },
    ]);
  });

  it("rejects invalid destinations before any worktree is created", async () => {
    const { operations } = fixture();
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === "validate_task_worktrees") throw new Error("invalid");
      return native(command, args);
    }) as typeof invoke);
    await expect(operations.createTask(input)).rejects.toMatchObject({ code: "invalid_worktrees" });
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "git_worktree_add")).toBe(
      false
    );
  });

  it("reports partial creation without deleting anything or claiming success", async () => {
    const { operations } = fixture();
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === "git_worktree_add" && args?.repoPath === "/repos/web")
        throw new Error("secret-token");
      return native(command, args);
    }) as typeof invoke);
    await expect(operations.createTask(input)).rejects.toMatchObject({
      code: "create_failed",
      details: {
        completedMembers: [expect.objectContaining({ repoId: "api" })],
        attemptedMember: expect.objectContaining({ repoId: "web" }),
      },
    });
    expect(operations.getState().tasks).toEqual([]);
    expect(
      vi.mocked(invoke).mock.calls.some(([command]) => command === "git_worktree_remove")
    ).toBe(false);
  });

  it("retains a task when optional setup fails and reports the affected repo", async () => {
    const { operations } = fixture();
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) =>
      command === "install_node_deps"
        ? { status: "error", message: "secret-token" }
        : native(command, args)) as typeof invoke);
    const result = await operations.createTask(input);
    expect(result.warnings.map((warning) => warning.repoId)).toEqual(["api", "web"]);
    expect(JSON.stringify(result)).not.toContain("secret-token");
    expect(operations.getState().tasks).toHaveLength(1);
  });

  it("reports created resources when persistence fails without publishing the task", async () => {
    const { operations, save, publish } = fixture();
    save.mockRejectedValueOnce(new Error("disk full"));
    await expect(operations.createTask(input)).rejects.toMatchObject({
      code: "persist_failed",
      details: { task: expect.objectContaining({ members: expect.any(Array) }) },
    });
    expect(publish).not.toHaveBeenCalled();
  });

  it("serializes UI and CLI mutations, publishes after save, and recovers after failure", async () => {
    const { operations, save, publish } = fixture();
    const saved = deferred();
    save.mockImplementationOnce(() => saved.promise);
    const first = operations.updateWorkspace("w1", { name: "Renamed" });
    const second = operations.change((state) => ({ selectedWorkspaceId: state.workspaces[0].id }));
    await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
    expect(publish).not.toHaveBeenCalled();
    saved.resolve();
    await Promise.all([first, second]);
    expect(operations.workspace("w1").name).toBe("Renamed");
    save.mockRejectedValueOnce(new Error("disk full"));
    await expect(operations.updateWorkspace("w1", { name: "Lost" })).rejects.toMatchObject({
      code: "persist_failed",
    });
    await operations.updateWorkspace("w1", { name: "Recovered" });
    expect(operations.workspace("w1").name).toBe("Recovered");
  });

  it("retains all records after partial workspace deletion and supports explicit retry", async () => {
    const { operations } = fixture([task]);
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === "git_worktree_remove" && args?.repoPath === "/repos/web")
        throw new Error("dirty");
      return native(command, args);
    }) as typeof invoke);
    await expect(operations.deleteWorkspace("w1", { deleteWorktrees: true })).rejects.toMatchObject(
      { code: "delete_failed", details: { removed: [task.members[0]], failed: [task.members[1]] } }
    );
    expect(operations.getState().tasks).toEqual([task]);
    expect(archiveTaskNote).not.toHaveBeenCalled();
    vi.mocked(invoke).mockImplementation(native as typeof invoke);
    await operations.deleteWorkspace("w1", { deleteWorktrees: true, force: true });
    expect(invoke).toHaveBeenCalledWith(
      "git_worktree_remove",
      expect.objectContaining({ force: true })
    );
    expect(operations.getState().workspaces).toEqual([]);
    expect(operations.getState().tasks).toEqual([]);
    expect(archiveTaskNote).toHaveBeenCalledWith(DEFAULT_STATE.vault, task, true);
  });

  it("keeps worktrees and local configs when forgetting a task", async () => {
    const { operations } = fixture([task]);
    await operations.deleteTask("t1", { deleteWorktrees: false });
    expect(closeTaskSessions).toHaveBeenCalledWith("t1");
    expect(invoke).not.toHaveBeenCalled();
    expect(archiveTaskNote).toHaveBeenCalled();
  });

  it("does not delete resources if sessions cannot be closed", async () => {
    const { operations } = fixture([task]);
    vi.mocked(closeTaskSessions).mockRejectedValueOnce(new Error("busy"));
    await expect(operations.deleteTask("t1", { deleteWorktrees: true })).rejects.toMatchObject({
      code: "sessions_active",
    });
    expect(invoke).not.toHaveBeenCalled();
    expect(operations.task("t1")).toBe(task);
  });

  it("requires an explicit workspace deletion mode only when tasks exist", async () => {
    await expect(fixture([task]).operations.deleteWorkspace("w1")).rejects.toMatchObject({
      code: "invalid_params",
    });
    await expect(fixture().operations.deleteWorkspace("w1")).resolves.toMatchObject({
      data: { id: "w1" },
    });
  });
});
