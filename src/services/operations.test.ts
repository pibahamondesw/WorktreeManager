import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_STATE, AppState, Task, Workspace } from "../types";
import { Operations, CreateTaskInput } from "./operations";
import { archiveTaskNote, ensureTaskNote } from "./notes";
import { LinearService } from "./linear";
import { normalizeTasks } from "../utils";
import { taskNoteFileName } from "./notes";
import { closeTaskSessions } from "./taskSessions";
import { dismissTaskSetup, getTaskSetup } from "./taskSetup";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("./store", () => ({ persist: vi.fn() }));
vi.mock("./notes", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./notes")>()),
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
  if (command === "install_python_deps") return { status: "skipped_no_config" };
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
      if (command === "install_python_deps") {
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
      expect(invoke).toHaveBeenCalledWith("install_python_deps", {
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
    for (const member of result.data.members) {
      expect(invoke).toHaveBeenCalledWith("install_python_deps", { worktreePath: member.path });
      expect(result.setup?.find((entry) => entry.repoId === member.repoId)?.steps).toContainEqual({
        stage: "install_python_deps",
        status: "installed",
      });
    }
    expect(result.warnings).toEqual([]);
    const commands = vi.mocked(invoke).mock.calls.map(([command]) => command);
    expect(commands.indexOf("validate_task_worktrees")).toBeLessThan(
      commands.indexOf("git_worktree_add")
    );
    expect(commands).not.toContain("open_editor");
  });

  it("starts every worktree before waiting and reports failures only after all settle", async () => {
    const { operations } = fixture();
    const second = deferred();
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === "git_worktree_add") {
        if (args?.repoPath === "/repos/api") throw new Error("private details");
        await second.promise;
      }
      return native(command, args);
    }) as typeof invoke);
    let finished = false;
    const creating = operations.createTask(input).catch((error) => {
      finished = true;
      return error;
    });
    await vi.waitFor(() =>
      expect(
        vi.mocked(invoke).mock.calls.filter(([command]) => command === "git_worktree_add")
      ).toHaveLength(2)
    );
    expect(finished).toBe(false);
    expect(operations.getState().tasks).toEqual([]);
    second.resolve();
    expect(await creating).toMatchObject({
      code: "create_failed",
      details: {
        completedMembers: [expect.objectContaining({ repoId: "web" })],
        failedMembers: [expect.objectContaining({ repoId: "api" })],
      },
    });
  });

  it("opens after basic preparation, runs repo setups concurrently and releases the mutation queue", async () => {
    const { operations } = fixture();
    const installing = deferred();
    const opening = deferred();
    const ready = vi.fn(async (created: Task) => {
      expect(operations.task(created.id)).toEqual(created);
      expect(created.workspaceFilePath).toBe("/generated/workspace.code-workspace");
      expect(
        vi.mocked(invoke).mock.calls.filter(([command]) => command === "copy_local_configs")
      ).toHaveLength(2);
      await opening.promise;
      return true;
    });
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === "install_node_deps") await installing.promise;
      return native(command, args);
    }) as typeof invoke);
    const creating = operations.createTask(input, undefined, ready);
    await vi.waitFor(() => expect(ready).toHaveBeenCalledOnce());
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "doppler_setup")).toBe(
      false
    );
    opening.resolve();
    await vi.waitFor(() =>
      expect(
        vi.mocked(invoke).mock.calls.filter(([command]) => command === "install_node_deps")
      ).toHaveLength(2)
    );
    expect(
      vi.mocked(invoke).mock.calls.some(([command]) => command === "install_python_deps")
    ).toBe(false);
    const created = operations.getState().tasks[0];
    expect(getTaskSetup(created.id)).toMatchObject({ active: true, opened: true });
    dismissTaskSetup(created.id);
    await operations.change(() => ({ selectedWorkspaceId: null }));
    await expect(
      operations.deleteTask(created.id, { deleteWorktrees: true })
    ).rejects.toMatchObject({ code: "setup_active" });
    await expect(
      operations.deleteWorkspace("w1", { deleteWorktrees: false })
    ).rejects.toMatchObject({ code: "setup_active" });
    expect(closeTaskSessions).not.toHaveBeenCalled();
    installing.resolve();
    await creating;
    expect(getTaskSetup(created.id)?.active).toBe(false);
    await operations.deleteTask(created.id, { deleteWorktrees: false });
    expect(getTaskSetup(created.id)).toBeUndefined();
  });

  it("waits for configuration and note creation before announcing readiness", async () => {
    const { operations } = fixture();
    await operations.change(() => ({ vault: { enabled: true, path: "/vault" } }));
    const copying = deferred();
    const note = deferred();
    vi.mocked(ensureTaskNote).mockImplementationOnce(async () => {
      await note.promise;
      return "/note";
    });
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === "copy_local_configs") await copying.promise;
      return native(command, args);
    }) as typeof invoke);
    const ready = vi.fn().mockResolvedValue(true);
    const creating = operations.createTask(input, undefined, ready);
    await vi.waitFor(() => expect(ensureTaskNote).toHaveBeenCalled());
    expect(ready).not.toHaveBeenCalled();
    copying.resolve();
    await vi.waitFor(() =>
      expect(
        getTaskSetup(operations.getState().tasks[0].id)?.repos.every(
          (repo) => repo.steps[0].status === "completed"
        )
      ).toBe(true)
    );
    expect(ready).not.toHaveBeenCalled();
    note.resolve();
    await creating;
    expect(ready).toHaveBeenCalledOnce();
  });

  it("continues setup when opening fails and reports warnings instead of success", async () => {
    const { operations } = fixture();
    const progress = vi.fn();
    const result = await operations.createTask(input, progress, async () => {
      throw new Error("private");
    });
    expect(result.warnings).toContainEqual(expect.objectContaining({ stage: "open" }));
    expect(
      result.setup?.every((repo) => repo.steps.some((step) => step.stage === "install_python_deps"))
    ).toBe(true);
    expect(progress).toHaveBeenLastCalledWith("Setup completed with warnings");
    expect(getTaskSetup(result.data.id)).toMatchObject({ active: false, opened: false });
    expect(JSON.stringify(result)).not.toContain("private");
  });

  it.each(["error", "skipped_no_cli", "rejected"])(
    "reports %s during setup and continues other steps and repositories",
    async (status) => {
      const { operations } = fixture();
      const remainingSetup = deferred();
      const message = "pnpm CLI not found on PATH";
      vi.mocked(invoke).mockImplementation((async (
        command: string,
        args?: Record<string, unknown>
      ) => {
        if (command === "install_node_deps" && args?.worktreePath === "/wt/api/pedro/wor-80") {
          if (status === "rejected") throw new Error("private");
          return { status, message };
        }
        if (command === "install_python_deps") await remainingSetup.promise;
        return native(command, args);
      }) as typeof invoke);
      const creating = operations.createTask(input);
      await vi.waitFor(() => {
        const created = operations.getState().tasks[0];
        expect(created).toBeDefined();
        expect(getTaskSetup(created.id)).toMatchObject({
          active: true,
          repos: [
            {
              steps: expect.arrayContaining([
                expect.objectContaining({ stage: "install_node_deps", status: "error" }),
              ]),
            },
            {
              steps: expect.arrayContaining([
                expect.objectContaining({ stage: "install_node_deps", status: "completed" }),
              ]),
            },
          ],
        });
      });
      remainingSetup.resolve();
      const result = await creating;
      const setup = getTaskSetup(result.data.id)!;
      expect(setup.active).toBe(false);
      expect(setup.repos[0].steps.find((step) => step.stage === "install_node_deps")?.message).toBe(
        status === "rejected" ? undefined : message
      );
      expect(setup.warnings).toContainEqual(
        expect.objectContaining({
          repoId: "api",
          stage: "install_node_deps",
        })
      );
      expect(JSON.stringify(result)).not.toContain(message);
      expect(JSON.stringify(setup)).not.toContain("private");
      expect(result.data).toEqual(operations.getState().tasks[0]);
    }
  );

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

  it.each([
    ["install_node_deps", "error"],
    ["install_python_deps", "error"],
    ["install_python_deps", "skipped_no_cli"],
  ])("retains a task when %s returns %s and reports the affected repo", async (stage, status) => {
    const { operations } = fixture();
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) =>
      command === stage
        ? { status, message: "secret-token" }
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

describe("linkTaskIssue", () => {
  beforeEach(() => {
    vi.spyOn(LinearService.prototype, "getIssue").mockResolvedValue(input.linearIssue!);
  });

  async function configured(tasks = [task]) {
    const result = fixture(tasks);
    await result.operations.change((state) => ({
      workspaces: state.workspaces.map((workspace) => ({
        ...workspace,
        linearApiKey: "workspace-key",
      })),
    }));
    result.save.mockClear();
    result.publish.mockClear();
    return result;
  }

  it("persists the association and preserves all repositories and the note filename after reload", async () => {
    const { operations, save } = await configured();
    const updated = await operations.linkTaskIssue("t1", " WOR-80 ");
    expect(LinearService.prototype.getIssue).toHaveBeenCalledWith("WOR-80");
    expect(updated).toEqual({
      ...task,
      noteFileName: "feature.md",
      linearIssueId: "issue-id",
      linearIssueIdentifier: "WOR-80",
      linearIssueTitle: "Local CLI",
    });
    expect(save).toHaveBeenCalledOnce();
    expect(taskNoteFileName(normalizeTasks([updated])[0])).toBe("feature.md");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("retries the same link without writing and rejects a different issue", async () => {
    const { operations, save } = await configured();
    await operations.linkTaskIssue("t1", "WOR-80");
    await operations.linkTaskIssue("t1", "wor-80");
    await operations.linkTaskIssue("t1", "issue-id");
    await expect(operations.linkTaskIssue("t1", "WOR-99")).rejects.toMatchObject({
      code: "conflict",
    });
    expect(save).toHaveBeenCalledOnce();
  });

  it("rejects missing tasks, empty identifiers, and missing credentials", async () => {
    const { operations } = fixture([task]);
    await expect(operations.linkTaskIssue("missing", "WOR-80")).rejects.toMatchObject({
      code: "not_found",
    });
    await expect(operations.linkTaskIssue("t1", " ")).rejects.toMatchObject({
      code: "invalid_params",
    });
    await expect(operations.linkTaskIssue("t1", "WOR-80")).rejects.toMatchObject({
      code: "linear_not_configured",
    });
    expect(LinearService.prototype.getIssue).not.toHaveBeenCalled();
  });

  it("retains the original task on lookup or persistence failure without exposing credentials", async () => {
    const { operations, save, publish } = await configured();
    vi.mocked(LinearService.prototype.getIssue).mockRejectedValueOnce(new Error("workspace-key"));
    await expect(operations.linkTaskIssue("t1", "WOR-80")).rejects.toMatchObject({
      code: "linear_issue_unavailable",
      message: expect.not.stringContaining("workspace-key"),
    });
    expect(save).not.toHaveBeenCalled();
    save.mockRejectedValueOnce(new Error("disk full"));
    await expect(operations.linkTaskIssue("t1", "WOR-80")).rejects.toMatchObject({
      code: "persist_failed",
    });
    expect(operations.task("t1")).toBe(task);
    expect(publish).not.toHaveBeenCalled();
  });

  it("serializes competing links so only the first succeeds", async () => {
    const { operations } = await configured();
    const results = await Promise.allSettled([
      operations.linkTaskIssue("t1", "WOR-80"),
      operations.linkTaskIssue("t1", "WOR-99"),
    ]);
    expect(results.map((result) => result.status)).toEqual(["fulfilled", "rejected"]);
    expect(LinearService.prototype.getIssue).toHaveBeenCalledOnce();
  });
});
