import { githubPrStatus, refreshGithubPrs, resetGithubCache } from "./github";
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
import { ResolvedScripts, resetScriptApprovalsCache } from "./repoScripts";
import { loadScriptApprovals, persist } from "./store";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("./store", () => ({
  persist: vi.fn(),
  loadScriptApprovals: vi.fn().mockResolvedValue({}),
}));
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
  if (command === "resolve_repo_scripts") return noScripts;
  if (command === "worktree_include_paths") return null;
  return undefined;
}

const noScripts: ResolvedScripts = {
  present: true,
  source: null,
  setup: null,
  setupHash: null,
  teardown: null,
  teardownHash: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  resetScriptApprovalsCache();
  vi.mocked(loadScriptApprovals).mockResolvedValue({});
  vi.mocked(invoke).mockImplementation(native as typeof invoke);
  vi.mocked(closeTaskSessions).mockResolvedValue(undefined);
});

describe("shared operations", () => {
  it("saves an enabled vault before installing Codex instructions", async () => {
    const { operations, save } = fixture();
    const vault = { enabled: true, path: "/custom/vault" };
    vi.mocked(invoke).mockImplementation((async (command: string) => {
      if (command === "sync_vault_agent") {
        expect(save).toHaveBeenCalledWith([["vault", vault]]);
        expect(operations.getState().vault).toEqual(vault);
      }
    }) as typeof invoke);
    await operations.updateVault(vault);
    expect(invoke).toHaveBeenCalledWith("sync_vault_agent", {
      agent: "codex",
      vaultPath: vault.path,
      repair: false,
    });
    expect(invoke).toHaveBeenCalledWith("sync_vault_agent", {
      agent: "claude",
      vaultPath: vault.path,
      repair: false,
    });
  });

  it("retains the vault on Codex setup failure and repairs without rewriting state", async () => {
    const { operations, save } = fixture();
    const vault = { enabled: true, path: "/vault" };
    vi.mocked(invoke).mockRejectedValueOnce("Permission denied");
    await expect(operations.updateVault(vault)).rejects.toThrow(
      "Vault settings were saved, but agent setup failed: Codex: Permission denied"
    );
    expect(operations.getState().vault).toEqual(vault);
    expect(invoke).toHaveBeenCalledWith("sync_vault_agent", {
      agent: "claude",
      vaultPath: "/vault",
      repair: false,
    });
    await operations.repairVaultAgents("codex");
    expect(invoke).toHaveBeenLastCalledWith("sync_vault_agent", {
      agent: "codex",
      vaultPath: "/vault",
      repair: true,
    });
    expect(save).toHaveBeenCalledTimes(1);
  });

  it("reports failures for both agents without rolling back the enabled vault", async () => {
    const { operations } = fixture();
    const vault = { enabled: true, path: "/vault" };
    vi.mocked(invoke).mockRejectedValueOnce("Codex denied").mockRejectedValueOnce("Claude denied");
    await expect(operations.updateVault(vault)).rejects.toThrow(
      "Codex: Codex denied; Claude Code: Claude denied"
    );
    expect(operations.getState().vault).toEqual(vault);
    await operations.repairVaultAgents();
    expect(invoke).toHaveBeenCalledWith("sync_vault_agent", {
      agent: "codex",
      vaultPath: "/vault",
      repair: true,
    });
    expect(invoke).toHaveBeenLastCalledWith("sync_vault_agent", {
      agent: "claude",
      vaultPath: "/vault",
      repair: true,
    });
  });

  it("keeps successful Codex setup when Claude fails and repairs only Claude on request", async () => {
    const { operations, save } = fixture();
    vi.mocked(invoke)
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce("Claude permission denied");
    await expect(operations.updateVault({ enabled: true, path: "/vault" })).rejects.toThrow(
      "agent setup failed: Claude Code: Claude permission denied"
    );
    vi.mocked(invoke).mockClear();
    await operations.repairVaultAgents("claude");
    expect(invoke).toHaveBeenCalledExactlyOnceWith("sync_vault_agent", {
      agent: "claude",
      vaultPath: "/vault",
      repair: true,
    });
    expect(save).toHaveBeenCalledOnce();
  });

  it("does not touch Codex instructions if vault persistence fails", async () => {
    const { operations, save } = fixture();
    save.mockRejectedValueOnce(new Error("Disk full"));
    await expect(operations.updateVault({ enabled: true, path: "/vault" })).rejects.toMatchObject({
      code: "persist_failed",
    });
    expect(invoke).not.toHaveBeenCalled();
    expect(operations.getState().vault).toEqual(DEFAULT_STATE.vault);
  });

  it("disables the vault and can retry cleanup without discarding the path", async () => {
    const { operations } = fixture();
    const vault = { enabled: false, path: "/vault" };
    vi.mocked(invoke).mockRejectedValueOnce("Permission denied");
    await expect(operations.updateVault(vault)).rejects.toThrow("agent cleanup failed: Codex:");
    expect(operations.getState().vault).toEqual(vault);
    expect(invoke).toHaveBeenCalledWith("sync_vault_agent", {
      agent: "claude",
      vaultPath: null,
      repair: false,
    });
    await operations.repairVaultAgents("codex");
    expect(invoke).toHaveBeenLastCalledWith("sync_vault_agent", {
      agent: "codex",
      vaultPath: null,
      repair: true,
    });
  });

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
    const creating = operations.createTask(input).catch((error: unknown) => {
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

  it("prepares Python environments before opening and tolerates preparation failures", async () => {
    const { operations } = fixture();
    const preparing = deferred();
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === "prepare_python_env") {
        await preparing.promise;
        throw new Error("private");
      }
      return native(command, args);
    }) as typeof invoke);
    const ready = vi.fn().mockResolvedValue(true);
    const creating = operations.createTask(input, undefined, ready);
    await vi.waitFor(() =>
      expect(
        vi.mocked(invoke).mock.calls.filter(([command]) => command === "prepare_python_env")
      ).toHaveLength(2)
    );
    expect(ready).not.toHaveBeenCalled();
    preparing.resolve();
    const result = await creating;
    expect(ready).toHaveBeenCalledOnce();
    expect(result.warnings).toEqual([]);
    expect(
      vi.mocked(invoke).mock.calls.filter(([command]) => command === "install_python_deps")
    ).toHaveLength(2);
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
              ]) as unknown,
            },
            {
              steps: expect.arrayContaining([
                expect.objectContaining({ stage: "install_node_deps", status: "completed" }),
              ]) as unknown,
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
        attemptedMember: { repoId: "web" },
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
      details: { task: { members: expect.any(Array) as unknown } },
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

describe("repository scripts", () => {
  const declared = (overrides: Partial<ResolvedScripts>): ResolvedScripts => ({
    ...noScripts,
    source: "worktreemanager",
    ...overrides,
  });

  function withScripts(resolved: ResolvedScripts, exitCode: number | null = 0) {
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === "resolve_repo_scripts")
        return String(args?.worktreePath).startsWith("/wt/api") ? resolved : noScripts;
      if (command === "run_task_script") return exitCode;
      return native(command, args);
    }) as typeof invoke);
  }

  const calls = (command: string) =>
    vi
      .mocked(invoke)
      .mock.calls.filter(([name]) => name === command)
      .map(([name, args]) => [name, args as Record<string, unknown> | undefined] as const);

  it("runs repository setup automatically instead of detected steps and passes context as env", async () => {
    withScripts(declared({ setup: "pnpm install", setupHash: "setup-hash" }));
    const { operations } = fixture();
    const result = await operations.createTask(input);
    const created = result.data;
    const [, run] = calls("run_task_script")[0];
    expect(run).toMatchObject({
      taskId: created.id,
      session: "setup:api",
      script: "pnpm install",
      cwd: created.members[0].path,
    });
    expect(Object.fromEntries((run as { env: [string, string][] }).env)).toMatchObject({
      WTM_ROOT_PATH: "/repos/api",
      WTM_WORKTREE_PATH: created.members[0].path,
      WTM_ISSUE_ID: "WOR-80",
      CONDUCTOR_ROOT_PATH: "/repos/api",
    });
    const detectedFor = (path: string) =>
      ["doppler_setup", "install_node_deps", "install_python_deps", "prepare_python_env"].some(
        (command) => calls(command).some(([, args]) => args?.worktreePath === path)
      );
    expect(detectedFor(created.members[0].path)).toBe(false);
    expect(detectedFor(created.members[1].path)).toBe(true);
    expect(result.setup?.[0].steps).toEqual([
      { stage: "config", status: "completed" },
      { stage: "script", status: "completed" },
    ]);
    expect(getTaskSetup(created.id)?.repos[0].source).toBe("worktreemanager");
    expect(result.warnings).toEqual([]);
  });

  it("copies .worktreeinclude matches instead of the default env files", async () => {
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === "worktree_include_paths")
        return args?.repoPath === "/repos/api" ? ["config/master.key"] : null;
      return native(command, args);
    }) as typeof invoke);
    const { operations } = fixture();
    await operations.createTask(input);
    const paths = (repo: string) =>
      calls("copy_local_configs").find(([, args]) => args?.sourceRepo === repo)?.[1]?.paths;
    expect(paths("/repos/api")).toContain("config/master.key");
    expect(paths("/repos/api")).not.toContain(".env");
    expect(paths("/repos/web")).toContain(".env");
  });

  it.each(["worktreemanager", "conductor", "superset", "cursor"] as const)(
    "runs new and changed %s setup scripts without approval",
    async (source) => {
      withScripts(declared({ source, setup: "make setup", setupHash: "setup-hash" }));
      const { operations } = fixture();
      const result = await operations.createTask(input);
      expect(calls("run_task_script")).toHaveLength(1);
      expect(result.setup?.[0].steps).toContainEqual({ stage: "script", status: "completed" });
      expect(result.warnings).toEqual([]);
      expect(getTaskSetup(result.data.id)?.repos[0].source).toBe(source);

      withScripts(declared({ source, setup: "make new-setup", setupHash: "changed-hash" }));
      const rerun = await operations.rerunRepoSetup(result.data.id, "api");
      expect(calls("run_task_script")).toHaveLength(2);
      expect(calls("run_task_script")[1][1]?.script).toBe("make new-setup");
      expect(loadScriptApprovals).not.toHaveBeenCalled();
      expect(persist).not.toHaveBeenCalled();
      expect(rerun.data.steps).toEqual([{ stage: "script", status: "completed" }]);
      expect(getTaskSetup(result.data.id)).toMatchObject({ active: false, warnings: [] });
    }
  );

  it("prefers the app override over repository setup", async () => {
    withScripts(declared({ setup: "make setup", setupHash: "setup-hash" }));
    const { operations } = fixture();
    await operations.updateWorkspace("w1", {
      repos: workspace.repos.map((repo) =>
        repo.id === "api" ? { ...repo, scripts: { setup: "make local-setup" } } : repo
      ),
    });
    const result = await operations.createTask(input);
    expect(calls("run_task_script")).toHaveLength(1);
    expect(calls("run_task_script")[0][1]?.script).toBe("make local-setup");
    expect(getTaskSetup(result.data.id)?.repos[0].source).toBe("local");
    expect(result.warnings).toEqual([]);
  });

  it("can fall back to detected setup instead of a repository script", async () => {
    withScripts(declared({ setup: "make setup", setupHash: "setup-hash" }));
    const { operations } = fixture();
    const created = (await operations.createTask(input)).data;
    vi.mocked(invoke).mockClear();
    const rerun = await operations.rerunRepoSetup(created.id, "api", "detected");
    expect(calls("run_task_script")).toEqual([]);
    expect(rerun.data.steps.map((step) => step.stage)).toEqual([
      "doppler_setup",
      "install_node_deps",
      "install_python_deps",
    ]);
  });

  it("trusts app overrides and reports failing or stopped scripts as warnings", async () => {
    const { operations } = fixture();
    await operations.updateWorkspace("w1", {
      repos: workspace.repos.map((repo) =>
        repo.id === "web" ? { ...repo, scripts: { setup: "exit 3" } } : repo
      ),
    });
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => (command === "run_task_script" ? 3 : native(command, args))) as typeof invoke);
    const result = await operations.createTask(input);
    expect(calls("run_task_script")).toHaveLength(1);
    expect(result.setup?.[1].steps).toContainEqual({ stage: "script", status: "error" });
    expect(getTaskSetup(result.data.id)?.repos[1]).toMatchObject({
      source: "local",
    });
    expect(getTaskSetup(result.data.id)?.repos[1].steps).toContainEqual({
      stage: "script",
      status: "error",
      message: "Setup script exited with code 3.",
      run: 1,
    });
    expect(result.warnings).toEqual([expect.objectContaining({ stage: "script", repoId: "web" })]);

    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => (command === "run_task_script" ? null : native(command, args))) as typeof invoke);
    const stopped = await operations.rerunRepoSetup(result.data.id, "web");
    expect(stopped.data.steps).toEqual([{ stage: "script", status: "cancelled" }]);
  });

  it("reports an invalid repository config without running detected setup", async () => {
    vi.mocked(invoke).mockImplementation((async (
      command: string,
      args?: Record<string, unknown>
    ) => {
      if (command === "resolve_repo_scripts" && String(args?.worktreePath).startsWith("/wt/api"))
        throw "Invalid .worktreemanager.toml: expected a table";
      return native(command, args);
    }) as typeof invoke);
    const { operations } = fixture();
    const result = await operations.createTask(input);
    expect(result.setup?.[0].steps).toContainEqual({ stage: "script", status: "error" });
    expect(
      calls("install_node_deps").some(([, args]) =>
        String(args?.worktreePath).startsWith("/wt/api")
      )
    ).toBe(false);
    expect(result.warnings).toContainEqual({
      stage: "script",
      repoId: "api",
      message: "Invalid .worktreemanager.toml: expected a table",
    });
  });

  it("requires approval for repository teardown and retains the task until it is given", async () => {
    withScripts(declared({ teardown: "make stop", teardownHash: "teardown-hash" }));
    const { operations } = fixture([task]);
    await expect(operations.deleteTask("t1", { deleteWorktrees: true })).rejects.toMatchObject({
      code: "teardown_needs_approval",
      details: { scripts: [{ repoName: "api", source: "worktreemanager", script: "make stop" }] },
    });
    expect(calls("git_worktree_remove")).toEqual([]);
    expect(operations.task("t1")).toBe(task);

    await operations.deleteTask("t1", { deleteWorktrees: true, approveTeardown: true });
    expect(persist).toHaveBeenCalledWith([
      ["scriptApprovals", { "/repos/api": ["teardown-hash"] }],
    ]);
    const order = vi.mocked(invoke).mock.calls.map(([name]) => name);
    expect(order.indexOf("run_task_script")).toBeLessThan(order.indexOf("git_worktree_remove"));
    expect(calls("run_task_script")[0][1]).toMatchObject({
      session: "teardown:api",
      script: "make stop",
      cwd: "/wt/api/feature",
      timeoutSecs: 300,
    });
    expect(operations.getState().tasks).toEqual([]);
  });

  it("retains the task when teardown fails and can delete without it", async () => {
    vi.mocked(loadScriptApprovals).mockResolvedValue({ "/repos/api": ["teardown-hash"] });
    withScripts(declared({ teardown: "make stop", teardownHash: "teardown-hash" }), 2);
    const { operations } = fixture([task]);
    await expect(operations.deleteTask("t1", { deleteWorktrees: true })).rejects.toMatchObject({
      code: "teardown_failed",
      details: { repoNames: ["api"] },
    });
    expect(calls("git_worktree_remove")).toEqual([]);
    expect(operations.task("t1")).toBe(task);
    expect(getTaskSetup("t1")?.active).toBe(false);

    vi.mocked(invoke).mockClear();
    await operations.deleteTask("t1", { deleteWorktrees: true, skipTeardown: true });
    expect(calls("run_task_script")).toEqual([]);
    expect(calls("git_worktree_remove")).toHaveLength(2);
    expect(operations.getState().tasks).toEqual([]);
  });

  it("skips teardown for worktrees that no longer exist", async () => {
    withScripts({ ...declared({ teardown: "make stop", teardownHash: "h" }), present: false });
    const { operations } = fixture([task]);
    await operations.deleteTask("t1", { deleteWorktrees: true });
    expect(calls("run_task_script")).toEqual([]);
    expect(operations.getState().tasks).toEqual([]);
  });
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

describe("linkTaskIssue", () => {
  beforeEach(() => {
    vi.spyOn(LinearService.prototype, "getIssue").mockResolvedValue(input.linearIssue!);
  });

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
      message: expect.not.stringContaining("workspace-key") as unknown,
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

describe("renameTask", () => {
  const linked = { ...task, linearIssueId: "issue-id", linearIssueTitle: "Old title" };

  beforeEach(() => {
    vi.spyOn(LinearService.prototype, "updateIssueTitle").mockResolvedValue();
  });

  it("updates Linear before persisting the trimmed title", async () => {
    const { operations, save } = await configured([linked]);
    const updated = await operations.renameTask("t1", "  New title ");
    expect(LinearService.prototype.updateIssueTitle).toHaveBeenCalledWith("issue-id", "New title");
    expect(updated).toEqual({ ...linked, linearIssueTitle: "New title" });
    expect(save).toHaveBeenCalledOnce();
  });

  it("skips Linear and the write when the title is unchanged", async () => {
    const { operations, save } = await configured([linked]);
    await expect(operations.renameTask("t1", " Old title ")).resolves.toBe(linked);
    expect(LinearService.prototype.updateIssueTitle).not.toHaveBeenCalled();
    expect(save).not.toHaveBeenCalled();
  });

  it("rejects empty titles, unlinked tasks, and missing credentials", async () => {
    const { operations } = fixture([linked, { ...task, id: "t2" }]);
    await expect(operations.renameTask("t1", " ")).rejects.toMatchObject({
      code: "invalid_params",
    });
    await expect(operations.renameTask("t2", "Title")).rejects.toMatchObject({
      code: "invalid_params",
    });
    await expect(operations.renameTask("t1", "Title")).rejects.toMatchObject({
      code: "linear_not_configured",
    });
    expect(LinearService.prototype.updateIssueTitle).not.toHaveBeenCalled();
  });

  it("retains the local title when Linear rejects the update, without exposing credentials", async () => {
    const { operations, save, publish } = await configured([linked]);
    vi.mocked(LinearService.prototype.updateIssueTitle).mockRejectedValueOnce(
      new Error("workspace-key")
    );
    await expect(operations.renameTask("t1", "New title")).rejects.toMatchObject({
      code: "linear_update_failed",
      message: expect.not.stringContaining("workspace-key") as unknown,
    });
    expect(save).not.toHaveBeenCalled();
    expect(publish).not.toHaveBeenCalled();
    expect(operations.task("t1").linearIssueTitle).toBe("Old title");
  });
});

describe("pinned tasks", () => {
  it("persists pins, reorders within a workspace, and restores the original unpinned order", async () => {
    const other = { ...task, id: "other", workspaceId: "w2", pinOrder: 0 };
    const { operations, save } = fixture([task, { ...task, id: "t2" }, other]);
    await Promise.all([operations.setTaskPinned("t2", true), operations.setTaskPinned("t1", true)]);
    expect(operations.task("t2").pinOrder).toBe(0);
    expect(operations.task("t1").pinOrder).toBe(1);
    await operations.setTaskPinned("t1", true);
    expect(operations.task("t1").pinOrder).toBe(1);
    await operations.reorderPinnedTasks("t1", "t2");
    const restored = normalizeTasks(operations.getState().tasks);
    expect(restored.map((item) => item.pinOrder)).toEqual([0, 1, 0]);
    expect(operations.task("other")).toBe(other);
    await operations.setTaskPinned("t1", false);
    await operations.setTaskPinned("t2", false);
    expect(operations.getState().tasks.map((item) => item.id)).toEqual(["t1", "t2", "other"]);
    expect(operations.task("t1").pinOrder).toBeUndefined();
    expect(save).toHaveBeenCalled();
  });

  it("rejects cross-workspace and unpinned drops", async () => {
    const { operations, save } = fixture([
      { ...task, pinOrder: 0 },
      { ...task, id: "other", workspaceId: "w2", pinOrder: 0 },
      { ...task, id: "unpinned" },
    ]);
    await expect(operations.reorderPinnedTasks("t1", "other")).rejects.toMatchObject({
      code: "invalid_params",
    });
    await expect(operations.reorderPinnedTasks("t1", "unpinned")).rejects.toMatchObject({
      code: "invalid_params",
    });
    await expect(operations.setTaskPinned("missing", true)).rejects.toMatchObject({
      code: "not_found",
    });
    expect(save).not.toHaveBeenCalled();
  });

  it("retains pin state when persistence fails", async () => {
    const { operations, save } = fixture([task, { ...task, id: "t2", pinOrder: 0 }]);
    save.mockRejectedValue(new Error("disk full"));
    await expect(operations.setTaskPinned("t1", true)).rejects.toMatchObject({
      code: "persist_failed",
    });
    await expect(operations.setTaskPinned("t2", false)).rejects.toMatchObject({
      code: "persist_failed",
    });
    expect(operations.task("t1").pinOrder).toBeUndefined();
    expect(operations.task("t2").pinOrder).toBe(0);
  });
});

it("refreshes existing projects, persists them through normalization and clears removed projects", async () => {
  const linked = { ...task, linearIssueId: "issue-id" };
  const { operations, save } = fixture([linked]);
  await operations.updateWorkspace(workspace.id, { linearApiKey: "test-key" });
  const fetch = vi.spyOn(LinearService.prototype, "fetchIssueLinearInfoBatch").mockResolvedValue({
    "issue-id": { status: null, prs: [], project: { id: "p1", name: "API" } },
  });
  await operations.refreshTaskProjects(workspace.id);
  expect(normalizeTasks(operations.getState().tasks)[0]).toMatchObject({
    linearProjectId: "p1",
    linearProjectName: "API",
  });
  save.mockClear();
  await operations.refreshTaskProjects(workspace.id);
  expect(save).not.toHaveBeenCalled();
  fetch.mockResolvedValueOnce({});
  await operations.refreshTaskProjects(workspace.id);
  expect(operations.getState().tasks[0].linearProjectId).toBe("p1");
  fetch.mockResolvedValueOnce({ "issue-id": { status: null, prs: [], project: null } });
  await operations.refreshTaskProjects(workspace.id);
  expect(operations.getState().tasks[0].linearProjectId).toBeUndefined();
  expect(operations.getState().tasks[0].linearProjectName).toBeUndefined();
  fetch.mockRestore();
});

describe("markPrReady", () => {
  const pr = {
    repoSlug: "org/repo",
    number: 1,
    state: "draft",
    title: "One",
    url: "https://github.com/org/repo/pull/1",
  };
  const status = { state: "open", isDraft: true, ci: "passing", review: "pending" };
  let operations: Operations;
  let fetchInfo: ReturnType<typeof vi.fn<LinearService["fetchIssueLinearInfoBatch"]>>;

  beforeEach(async () => {
    resetGithubCache();
    const state = {
      ...DEFAULT_STATE,
      workspaces: [{ ...workspace, linearApiKey: "test-key" }],
      tasks: [{ ...task, linearIssueId: "i1" }],
    };
    operations = new Operations(
      () => state,
      vi.fn(),
      () => "cursor",
      vi.fn()
    );
    fetchInfo = vi
      .spyOn(LinearService.prototype, "fetchIssueLinearInfoBatch")
      .mockResolvedValue({ i1: { status: null, prs: [pr] } });
    vi.mocked(invoke).mockImplementation((async (command: string) =>
      command === "github_pr_status_batch" ? { 1: status } : undefined) as typeof invoke);
    await refreshGithubPrs([pr]);
    vi.mocked(invoke).mockClear();
  });

  it("validates the link, shares duplicate actions and refreshes the affected repo", async () => {
    const first = operations.markPrReady(task.id, pr);
    expect(operations.markPrReady(task.id, pr)).toBe(first);
    await first;
    expect(fetchInfo).toHaveBeenCalledWith(["i1"]);
    expect(invoke).toHaveBeenCalledWith("github_pr_ready", { repoSlug: "org/repo", prNumber: 1 });
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("refreshes PRs from other tasks in the same repository after ready", async () => {
    await refreshGithubPrs([{ ...pr, number: 2 }]);
    vi.mocked(invoke).mockClear();
    await operations.markPrReady(task.id, pr);
    expect(invoke).toHaveBeenLastCalledWith("github_pr_status_batch", {
      repoSlug: "org/repo",
      prNumbers: [1, 2],
    });
  });

  it("rejects a removed attachment before mutating GitHub", async () => {
    fetchInfo.mockResolvedValue({ i1: { status: null, prs: [] } });
    await expect(operations.markPrReady(task.id, pr)).rejects.toThrow("no longer linked");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("retains draft when ready fails", async () => {
    vi.mocked(invoke).mockRejectedValueOnce(new Error("denied"));
    await expect(operations.markPrReady(task.id, pr)).rejects.toThrow("denied");
    expect(githubPrStatus(pr)?.isDraft).toBe(true);
    expect(invoke).toHaveBeenCalledOnce();
  });

  it("preserves success when the subsequent refresh fails", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("offline"));
    await expect(operations.markPrReady(task.id, pr)).resolves.toBeUndefined();
    expect(githubPrStatus(pr)).toBeUndefined();
  });

  it("rejects unverified or inactive PRs", async () => {
    resetGithubCache();
    await expect(operations.markPrReady(task.id, pr)).rejects.toThrow("Refresh the pull request");
    expect(invoke).not.toHaveBeenCalled();
  });
});
