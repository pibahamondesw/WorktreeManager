import { taskLinearStore } from "./taskLinear";
import { githubPrStatus, refreshGithubPrs, resetGithubCache } from "./github";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_STATE, AppState, EditorApp, Task, Workspace } from "../types";
import { Operations, CreateTaskInput } from "./operations";
import { openEditorForWorktree } from "./openEditor";
import { archiveTaskNote, ensureTaskNote } from "./notes";
import { LinearService } from "./linear";
import { normalizeTasks } from "../utils";
import { taskNoteFileName } from "./notes";
import { closeTaskSessions } from "./taskSessions";
import { DetectedStep, dismissTaskSetup, getTaskSetup } from "./taskSetup";
import { ResolvedScripts, resetScriptApprovalsCache } from "./repoScripts";
import { loadScriptApprovals, persist } from "./store";
import { MemoryWorktreeLifecycle } from "./memoryWorktreeLifecycle";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  Channel: class {
    onmessage?: (message: unknown) => void;
  },
}));
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
const apiPath = "/wt/api/pedro/wor-80";
const webPath = "/wt/web/pedro/wor-80";

const detectedOutcomes: DetectedStep[] = [
  { stage: "doppler", status: "skipped" },
  { stage: "node", status: "completed" },
  { stage: "python", status: "skipped" },
];

function fixture(tasks: Task[] = [], editor: EditorApp = "cursor") {
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
  const lifecycle = new MemoryWorktreeLifecycle().seed(tasks);
  lifecycle.detectedSteps = () => detectedOutcomes;
  const operations = new Operations(
    () => state,
    publish,
    () => editor,
    save,
    lifecycle
  );
  return { operations, save, publish, lifecycle };
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
  taskLinearStore.reset();
  vi.clearAllMocks();
  resetScriptApprovalsCache();
  vi.mocked(loadScriptApprovals).mockResolvedValue({});
  vi.mocked(invoke).mockResolvedValue(undefined);
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
    const { operations, lifecycle } = fixture();
    const releaseSetup = lifecycle.hold("runDetectedSetup");
    let complete = false;
    const creating = operations.createTask(input).then((result) => {
      complete = true;
      return result;
    });
    await vi.waitFor(() =>
      expect(lifecycle.targets("runDetectedSetup")).toEqual([apiPath, webPath])
    );
    expect(complete).toBe(false);
    expect(operations.getState().tasks).toHaveLength(1);
    releaseSetup();
    const result = await creating;
    expect(result.data.members).toHaveLength(2);
    expect(result.data.linearIssueId).toBe("issue-id");
    expect(result.data.workspaceFilePath).toBeNull();
    expect([...lifecycle.worktrees.keys()]).toEqual([apiPath, webPath]);
    expect(result.setup).toHaveLength(2);
    for (const member of result.data.members)
      expect(result.setup?.find((entry) => entry.repoId === member.repoId)?.steps).toContainEqual({
        stage: "python",
        status: "skipped",
      });
    expect(result.warnings).toEqual([]);
    const steps = lifecycle.calls.map((call) => call.step);
    expect(steps.indexOf("validateWorktrees")).toBeLessThan(steps.indexOf("addWorktree"));
    expect(invoke).not.toHaveBeenCalled();
  });

  it("marks the tasks an automation creates", async () => {
    const { operations } = fixture();
    const { data: manual } = await operations.createTask(input);
    expect(manual.automationId).toBeUndefined();
    const { data: created } = await operations.createTask({
      ...input,
      branchName: "wtm/pr-babysitter/1a2b3c4d",
      linearIssue: undefined,
      automationId: "pr-babysitter",
    });
    expect(operations.task(created.id).automationId).toBe("pr-babysitter");
  });

  it("creates with another editor, renames the workspace, opens Cursor and cleans by task ID", async () => {
    const { operations, lifecycle } = fixture([], "codex");
    const { data: created } = await operations.createTask(input);
    expect(lifecycle.copiedConfigs.get(apiPath)).toEqual([
      ".codex",
      ".agents",
      ".env",
      ".env.local",
      ".env.development",
      ".env.development.local",
    ]);
    await operations.updateWorkspace(workspace.id, { name: "Renamed payments" });
    const opened = {
      message: "Cursor opened",
      workspaceFile: `/workspaces/${created.id}.code-workspace`,
    };
    vi.mocked(invoke).mockResolvedValueOnce(opened);
    await expect(
      openEditorForWorktree(
        "cursor",
        created.members.map((member) => member.path),
        created.id,
        created.workspaceFilePath
      )
    ).resolves.toEqual(opened);
    expect(invoke).toHaveBeenCalledWith("open_editor", {
      editor: "cursor",
      folders: created.members.map((member) => member.path),
      taskId: created.id,
      legacyWorkspaceFilePath: null,
    });
    await operations.deleteTask(created.id, { deleteWorktrees: true });
    expect(lifecycle.removedTaskWorkspaces).toEqual([created]);
    expect(lifecycle.worktrees.size).toBe(0);
    expect(lifecycle.cleanedTasks).toEqual([created]);
    expect(operations.getState().tasks).toEqual([]);
  });

  it("starts every worktree before waiting and reports failures only after all settle", async () => {
    const { operations, lifecycle } = fixture();
    lifecycle.fail("addWorktree", (path) => path === apiPath);
    const releaseWeb = lifecycle.hold("addWorktree", (path) => path === webPath);
    let finished = false;
    const creating = operations.createTask(input).catch((error: unknown) => {
      finished = true;
      return error;
    });
    await vi.waitFor(() => expect(lifecycle.targets("addWorktree")).toHaveLength(2));
    expect(finished).toBe(false);
    expect(operations.getState().tasks).toEqual([]);
    releaseWeb();
    expect(await creating).toMatchObject({
      code: "create_failed",
      details: {
        completedMembers: [expect.objectContaining({ repoId: "web" })],
        failedMembers: [expect.objectContaining({ repoId: "api" })],
      },
    });
    expect([...lifecycle.worktrees.keys()]).toEqual([webPath]);
  });

  it("opens after basic preparation, runs repo setups concurrently and releases the mutation queue", async () => {
    const { operations, lifecycle } = fixture();
    const releaseSetup = lifecycle.hold("runDetectedSetup");
    let releaseOpening!: () => void;
    const opening = new Promise<void>((resolve) => {
      releaseOpening = resolve;
    });
    const ready = vi.fn(async (created: Task) => {
      expect(operations.task(created.id)).toEqual(created);
      expect(created.workspaceFilePath).toBeNull();
      expect(lifecycle.copiedConfigs.size).toBe(2);
      await opening;
      return true;
    });
    const creating = operations.createTask(input, undefined, ready);
    await vi.waitFor(() => expect(ready).toHaveBeenCalledOnce());
    expect(lifecycle.targets("runDetectedSetup")).toEqual([]);
    releaseOpening();
    await vi.waitFor(() => expect(lifecycle.targets("runDetectedSetup")).toHaveLength(2));
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
    releaseSetup();
    await creating;
    expect(getTaskSetup(created.id)?.active).toBe(false);
    await operations.deleteTask(created.id, { deleteWorktrees: false });
    expect(getTaskSetup(created.id)).toBeUndefined();
    expect(lifecycle.worktrees.size).toBe(2);
  });

  it("waits for configuration and note creation before announcing readiness", async () => {
    const { operations, lifecycle } = fixture();
    await operations.change(() => ({ vault: { enabled: true, path: "/vault" } }));
    const releaseCopy = lifecycle.hold("copyLocalConfigs");
    let releaseNote!: () => void;
    const note = new Promise<void>((resolve) => {
      releaseNote = resolve;
    });
    vi.mocked(ensureTaskNote).mockImplementationOnce(async () => {
      await note;
      return "/note";
    });
    const ready = vi.fn().mockResolvedValue(true);
    const creating = operations.createTask(input, undefined, ready);
    await vi.waitFor(() => expect(ensureTaskNote).toHaveBeenCalled());
    expect(ready).not.toHaveBeenCalled();
    releaseCopy();
    await vi.waitFor(() =>
      expect(
        getTaskSetup(operations.getState().tasks[0].id)?.repos.every(
          (repo) => repo.steps[0].status === "completed"
        )
      ).toBe(true)
    );
    expect(ready).not.toHaveBeenCalled();
    releaseNote();
    await creating;
    expect(ready).toHaveBeenCalledOnce();
  });

  it.each([false, true])(
    "waits for detected preparation before opening (failure: %s)",
    async (fails) => {
      const { operations, lifecycle } = fixture();
      const releasePreparation = lifecycle.hold("prepareDetectedSetup");
      if (fails) lifecycle.fail("prepareDetectedSetup");
      const ready = vi.fn().mockResolvedValue(true);
      const creating = operations.createTask(input, undefined, ready);
      await vi.waitFor(() => expect(lifecycle.targets("prepareDetectedSetup")).toHaveLength(2));
      expect(ready).not.toHaveBeenCalled();
      releasePreparation();
      const result = await creating;
      expect(ready).toHaveBeenCalledOnce();
      expect(result.warnings).toHaveLength(fails ? 2 : 0);
      expect(lifecycle.targets("runDetectedSetup")).toHaveLength(fails ? 0 : 2);
      expect(operations.getState().tasks).toHaveLength(1);
      expect(JSON.stringify(result)).not.toContain("fake");
      expect(getTaskSetup(result.data.id)?.active).toBe(false);
    }
  );

  it("continues setup when opening fails and reports warnings instead of success", async () => {
    const { operations } = fixture();
    const progress = vi.fn();
    const result = await operations.createTask(input, progress, async () => {
      throw new Error("private");
    });
    expect(result.warnings).toContainEqual(expect.objectContaining({ stage: "open" }));
    expect(result.setup?.every((repo) => repo.steps.some((step) => step.stage === "python"))).toBe(
      true
    );
    expect(progress).toHaveBeenLastCalledWith("Setup completed with warnings");
    expect(getTaskSetup(result.data.id)).toMatchObject({ active: false, opened: false });
    expect(JSON.stringify(result)).not.toContain("private");
  });

  it("rejects duplicate retries while detected setup is being prepared", async () => {
    const { operations, lifecycle } = fixture([task]);
    const releasePreparation = lifecycle.hold("prepareDetectedSetup");
    const retrying = operations.rerunRepoSetup(task.id, "api");
    await vi.waitFor(() =>
      expect(lifecycle.targets("prepareDetectedSetup")).toEqual([task.members[0].path])
    );
    await expect(operations.rerunRepoSetup(task.id, "api")).rejects.toMatchObject({
      code: "setup_active",
    });
    releasePreparation();
    await retrying;
    expect(getTaskSetup(task.id)?.active).toBe(false);
    expect(lifecycle.targets("prepareDetectedSetup")).toHaveLength(1);
  });

  it.each([false, true])(
    "reports typed setup errors and continues other repositories (transport failure: %s)",
    async (rejected) => {
      const { operations, lifecycle } = fixture();
      const message = "pnpm CLI not found on PATH";
      const failing = detectedOutcomes.map((step) =>
        step.stage === "node" ? { ...step, status: "error" as const, message } : step
      );
      lifecycle.detectedSteps = (path) => (path === apiPath ? failing : detectedOutcomes);
      if (rejected) lifecycle.fail("runDetectedSetup", (path) => path === apiPath);
      const releaseWeb = lifecycle.hold("runDetectedSetup", (path) => path === webPath);
      const creating = operations.createTask(input);
      await vi.waitFor(() => {
        const created = operations.getState().tasks[0];
        expect(created).toBeDefined();
        expect(getTaskSetup(created.id)).toMatchObject({
          active: true,
          repos: [
            {
              steps: expect.arrayContaining([
                expect.objectContaining({ stage: "node", status: "error" }),
              ]) as unknown,
            },
            {
              steps: expect.arrayContaining([
                expect.objectContaining({ stage: "node", status: "pending" }),
              ]) as unknown,
            },
          ],
        });
      });
      releaseWeb();
      const result = await creating;
      const setup = getTaskSetup(result.data.id)!;
      expect(setup.active).toBe(false);
      expect(setup.repos[0].steps.find((step) => step.stage === "node")?.message).toBe(
        rejected ? undefined : message
      );
      expect(setup.warnings).toContainEqual(
        expect.objectContaining({ repoId: "api", stage: "node" })
      );
      expect(result.warnings.filter((warning) => warning.stage === "node")).toHaveLength(1);
      expect(result.setup?.[1].steps).toContainEqual({ stage: "node", status: "completed" });
      expect(JSON.stringify(result)).not.toContain(message);
      expect(JSON.stringify(setup)).not.toContain("fake");
      expect(result.data).toEqual(operations.getState().tasks[0]);
    }
  );

  it("selects repositories and preserves manual branch resolution", async () => {
    const { operations, lifecycle } = fixture();
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
    expect([...lifecycle.worktrees.keys()]).toEqual(["/wt/web/user/feature"]);
  });

  it("rejects invalid destinations before any worktree is created", async () => {
    const { operations, lifecycle } = fixture();
    lifecycle.fail("validateWorktrees");
    await expect(operations.createTask(input)).rejects.toMatchObject({ code: "invalid_worktrees" });
    expect(lifecycle.worktrees.size).toBe(0);
    expect(operations.getState().tasks).toEqual([]);
  });

  it("rejects worktrees another task owns as a conflict", async () => {
    const owner = {
      ...task,
      members: task.members.map((member) => ({
        ...member,
        path: `/wt/${member.repoId}/pedro/wor-80`,
      })),
    };
    const { operations, lifecycle } = fixture([owner]);
    await expect(operations.createTask(input)).rejects.toMatchObject({ code: "conflict" });
    expect(lifecycle.targets("addWorktree")).toEqual([]);
    expect(operations.getState().tasks).toEqual([owner]);
  });

  it("reports partial creation without deleting anything or claiming success", async () => {
    const { operations, lifecycle } = fixture();
    lifecycle.fail("addWorktree", (path) => path === webPath);
    await expect(operations.createTask(input)).rejects.toMatchObject({
      code: "create_failed",
      details: {
        completedMembers: [expect.objectContaining({ repoId: "api" })],
        attemptedMember: { repoId: "web" },
      },
    });
    expect(operations.getState().tasks).toEqual([]);
    expect([...lifecycle.worktrees.keys()]).toEqual([apiPath]);
  });

  it.each(["doppler", "node", "python"] as const)(
    "retains a task when %s fails and reports the affected repos",
    async (stage) => {
      const { operations, lifecycle } = fixture();
      lifecycle.detectedSteps = () =>
        detectedOutcomes.map((step) =>
          step.stage === stage ? { ...step, status: "error", message: "secret-token" } : step
        );
      const result = await operations.createTask(input);
      expect(result.warnings.map((warning) => warning.repoId)).toEqual(["api", "web"]);
      expect(JSON.stringify(result)).not.toContain("secret-token");
      expect(operations.getState().tasks).toHaveLength(1);
    }
  );

  it("keeps the task when local configuration cannot be copied", async () => {
    const { operations, lifecycle } = fixture();
    lifecycle.fail("copyLocalConfigs", (path) => path === apiPath);
    const result = await operations.createTask(input);
    expect(result.warnings).toEqual([expect.objectContaining({ stage: "config", repoId: "api" })]);
    expect(result.setup?.[0].steps[0]).toEqual({ stage: "config", status: "error" });
    expect(result.setup?.[0].steps).toContainEqual({ stage: "node", status: "completed" });
    expect(operations.getState().tasks).toHaveLength(1);
  });

  it("reports created resources when persistence fails without publishing the task", async () => {
    const { operations, save, publish, lifecycle } = fixture();
    save.mockRejectedValueOnce(new Error("disk full"));
    await expect(operations.createTask(input)).rejects.toMatchObject({
      code: "persist_failed",
      details: { task: { members: expect.any(Array) as unknown } },
    });
    expect(publish).not.toHaveBeenCalled();
    expect(lifecycle.worktrees.size).toBe(2);
    expect(lifecycle.targets("copyLocalConfigs")).toEqual([]);
  });

  it("serializes UI and CLI mutations, publishes after save, and recovers after failure", async () => {
    const { operations, save, publish } = fixture();
    let releaseSave!: () => void;
    save.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          releaseSave = resolve;
        })
    );
    const first = operations.updateWorkspace("w1", { name: "Renamed" });
    const second = operations.change((state) => ({ selectedWorkspaceId: state.workspaces[0].id }));
    await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
    expect(publish).not.toHaveBeenCalled();
    releaseSave();
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
    const { operations, lifecycle } = fixture([task]);
    lifecycle.fail("removeWorktree", (path) => path.startsWith("/wt/web"));
    await expect(operations.deleteWorkspace("w1", { deleteWorktrees: true })).rejects.toMatchObject(
      { code: "delete_failed", details: { removed: [task.members[0]], failed: [task.members[1]] } }
    );
    expect(operations.getState().tasks).toEqual([task]);
    expect(operations.getState().workspaces).toEqual([workspace]);
    expect(archiveTaskNote).not.toHaveBeenCalled();
    expect(lifecycle.removedTaskWorkspaces).toEqual([]);
    expect(lifecycle.cleanedTasks).toEqual([]);
    lifecycle.succeed("removeWorktree");
    await operations.deleteWorkspace("w1", { deleteWorktrees: true, force: true });
    expect(lifecycle.worktrees.size).toBe(0);
    expect(operations.getState().workspaces).toEqual([]);
    expect(operations.getState().tasks).toEqual([]);
    expect(archiveTaskNote).toHaveBeenCalledWith(DEFAULT_STATE.vault, task, true);
  });

  it("keeps worktrees and local configs when forgetting a task", async () => {
    const { operations, lifecycle } = fixture([task]);
    await operations.deleteTask("t1", { deleteWorktrees: false });
    expect(closeTaskSessions).toHaveBeenCalledWith("t1");
    expect(lifecycle.worktrees.size).toBe(2);
    expect(lifecycle.removedTaskWorkspaces).toEqual([task]);
    expect(lifecycle.cleanedTasks).toEqual([]);
    expect(archiveTaskNote).toHaveBeenCalled();
  });

  it.each([false, true])(
    "cleans stable and legacy workspace paths (delete worktrees: %s)",
    async (deleteWorktrees) => {
      const legacy = { ...task, workspaceFilePath: "/workspaces/payments/feature.code-workspace" };
      const { operations, lifecycle } = fixture([legacy]);
      await operations.deleteTask(legacy.id, { deleteWorktrees });
      expect(lifecycle.removedTaskWorkspaces).toEqual([legacy]);
      expect(operations.getState().tasks).toEqual([]);
    }
  );

  it("reports workspace cleanup failures without retaining a deleted task", async () => {
    const { operations, lifecycle } = fixture([task]);
    lifecycle.fail("removeTaskWorkspace");
    const result = await operations.deleteTask(task.id, { deleteWorktrees: false });
    expect(result.warnings).toEqual([expect.objectContaining({ stage: "workspace_file" })]);
    expect(operations.getState().tasks).toEqual([]);
  });

  it("reports failed post-removal cleanups as warnings after deleting the task", async () => {
    const { operations, lifecycle } = fixture([task]);
    lifecycle.failingCleanups.push("doppler_cleanup");
    const result = await operations.deleteTask(task.id, { deleteWorktrees: true });
    expect(result.warnings).toEqual([expect.objectContaining({ stage: "doppler_cleanup" })]);
    expect(lifecycle.worktrees.size).toBe(0);
    expect(operations.getState().tasks).toEqual([]);
    expect(archiveTaskNote).toHaveBeenCalled();
  });

  it("does not delete resources if sessions cannot be closed", async () => {
    const { operations, lifecycle } = fixture([task]);
    vi.mocked(closeTaskSessions).mockRejectedValueOnce(new Error("busy"));
    await expect(operations.deleteTask("t1", { deleteWorktrees: true })).rejects.toMatchObject({
      code: "sessions_active",
    });
    expect(lifecycle.calls).toEqual([]);
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

  function scripted(resolved: ResolvedScripts, tasks: Task[] = [], exitCode: number | null = 0) {
    const result = fixture(tasks);
    result.lifecycle.scripts.set("/repos/api", resolved);
    result.lifecycle.exitCode = () => exitCode;
    return result;
  }

  it("runs repository setup automatically instead of detected steps and passes context as env", async () => {
    const { operations, lifecycle } = scripted(
      declared({ setup: "pnpm install", setupHash: "setup-hash" })
    );
    const result = await operations.createTask(input);
    const created = result.data;
    const [run] = lifecycle.scriptRuns;
    expect(run).toMatchObject({
      taskId: created.id,
      session: "setup:api",
      script: "pnpm install",
      cwd: created.members[0].path,
    });
    expect(Object.fromEntries(run.env)).toMatchObject({
      WTM_ROOT_PATH: "/repos/api",
      WTM_WORKTREE_PATH: created.members[0].path,
      WTM_ISSUE_ID: "WOR-80",
      CONDUCTOR_ROOT_PATH: "/repos/api",
    });
    expect(lifecycle.targets("prepareDetectedSetup")).toEqual([created.members[1].path]);
    expect(result.setup?.[0].steps).toEqual([
      { stage: "config", status: "completed" },
      { stage: "script", status: "completed" },
    ]);
    expect(getTaskSetup(created.id)?.repos[0].source).toBe("worktreemanager");
    expect(result.warnings).toEqual([]);
  });

  it("copies .worktreeinclude matches instead of the default env files", async () => {
    const { operations, lifecycle } = fixture();
    lifecycle.includes.set("/repos/api", ["config/master.key"]);
    await operations.createTask(input);
    expect(lifecycle.copiedConfigs.get(apiPath)).toContain("config/master.key");
    expect(lifecycle.copiedConfigs.get(apiPath)).not.toContain(".env");
    expect(lifecycle.copiedConfigs.get(webPath)).toContain(".env");
  });

  it.each(["worktreemanager", "conductor", "superset", "cursor"] as const)(
    "runs new and changed %s setup scripts without approval",
    async (source) => {
      const { operations, lifecycle } = scripted(
        declared({ source, setup: "make setup", setupHash: "setup-hash" })
      );
      const result = await operations.createTask(input);
      expect(lifecycle.scriptRuns).toHaveLength(1);
      expect(result.setup?.[0].steps).toContainEqual({ stage: "script", status: "completed" });
      expect(result.warnings).toEqual([]);
      expect(getTaskSetup(result.data.id)?.repos[0].source).toBe(source);

      lifecycle.scripts.set(
        "/repos/api",
        declared({ source, setup: "make new-setup", setupHash: "changed-hash" })
      );
      const rerun = await operations.rerunRepoSetup(result.data.id, "api");
      expect(lifecycle.scriptRuns.map((run) => run.script)).toEqual([
        "make setup",
        "make new-setup",
      ]);
      expect(loadScriptApprovals).not.toHaveBeenCalled();
      expect(persist).not.toHaveBeenCalled();
      expect(rerun.data.steps).toEqual([{ stage: "script", status: "completed" }]);
      expect(getTaskSetup(result.data.id)).toMatchObject({ active: false, warnings: [] });
    }
  );

  it("prefers the app override over repository setup", async () => {
    const { operations, lifecycle } = scripted(
      declared({ setup: "make setup", setupHash: "setup-hash" })
    );
    await operations.updateWorkspace("w1", {
      repos: workspace.repos.map((repo) =>
        repo.id === "api" ? { ...repo, scripts: { setup: "make local-setup" } } : repo
      ),
    });
    const result = await operations.createTask(input);
    expect(lifecycle.scriptRuns.map((run) => run.script)).toEqual(["make local-setup"]);
    expect(getTaskSetup(result.data.id)?.repos[0].source).toBe("local");
    expect(result.warnings).toEqual([]);
  });

  it("can fall back to detected setup instead of a repository script", async () => {
    const { operations, lifecycle } = scripted(
      declared({ setup: "make setup", setupHash: "setup-hash" })
    );
    const created = (await operations.createTask(input)).data;
    const rerun = await operations.rerunRepoSetup(created.id, "api", "detected");
    expect(lifecycle.scriptRuns).toHaveLength(1);
    expect(rerun.data.steps.map((step) => step.stage)).toEqual(["doppler", "node", "python"]);
  });

  it("trusts app overrides and reports failing or stopped scripts as warnings", async () => {
    const { operations, lifecycle } = fixture();
    await operations.updateWorkspace("w1", {
      repos: workspace.repos.map((repo) =>
        repo.id === "web" ? { ...repo, scripts: { setup: "exit 3" } } : repo
      ),
    });
    lifecycle.exitCode = () => 3;
    const result = await operations.createTask(input);
    expect(lifecycle.scriptRuns).toHaveLength(1);
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

    lifecycle.exitCode = () => null;
    const stopped = await operations.rerunRepoSetup(result.data.id, "web");
    expect(stopped.data.steps).toEqual([{ stage: "script", status: "cancelled" }]);
  });

  it("reports an invalid repository config without running detected setup", async () => {
    const { operations, lifecycle } = fixture();
    lifecycle.scripts.set("/repos/api", "Invalid .worktreemanager.toml: expected a table");
    const result = await operations.createTask(input);
    expect(result.setup?.[0].steps).toContainEqual({ stage: "script", status: "error" });
    expect(lifecycle.targets("runDetectedSetup")).toEqual([webPath]);
    expect(result.warnings).toContainEqual({
      stage: "script",
      repoId: "api",
      message: "Invalid .worktreemanager.toml: expected a table",
    });
  });

  it("requires approval for repository teardown and retains the task until it is given", async () => {
    const { operations, lifecycle } = scripted(
      declared({ teardown: "make stop", teardownHash: "teardown-hash" }),
      [task]
    );
    await expect(operations.deleteTask("t1", { deleteWorktrees: true })).rejects.toMatchObject({
      code: "teardown_needs_approval",
      details: { scripts: [{ repoName: "api", source: "worktreemanager", script: "make stop" }] },
    });
    expect(lifecycle.worktrees.size).toBe(2);
    expect(operations.task("t1")).toBe(task);

    await operations.deleteTask("t1", { deleteWorktrees: true, approveTeardown: true });
    expect(persist).toHaveBeenCalledWith([
      ["scriptApprovals", { "/repos/api": ["teardown-hash"] }],
    ]);
    const steps = lifecycle.calls.map((call) => call.step);
    expect(steps.indexOf("runScript")).toBeLessThan(steps.indexOf("removeWorktree"));
    expect(lifecycle.scriptRuns[0]).toMatchObject({
      session: "teardown:api",
      script: "make stop",
      cwd: "/wt/api/feature",
      timeoutSecs: 300,
    });
    expect(lifecycle.worktrees.size).toBe(0);
    expect(operations.getState().tasks).toEqual([]);
  });

  it("retains the task when teardown fails and can delete without it", async () => {
    vi.mocked(loadScriptApprovals).mockResolvedValue({ "/repos/api": ["teardown-hash"] });
    const { operations, lifecycle } = scripted(
      declared({ teardown: "make stop", teardownHash: "teardown-hash" }),
      [task],
      2
    );
    await expect(operations.deleteTask("t1", { deleteWorktrees: true })).rejects.toMatchObject({
      code: "teardown_failed",
      details: { repoNames: ["api"] },
    });
    expect(lifecycle.worktrees.size).toBe(2);
    expect(operations.task("t1")).toBe(task);
    expect(getTaskSetup("t1")?.active).toBe(false);

    await operations.deleteTask("t1", { deleteWorktrees: true, skipTeardown: true });
    expect(lifecycle.scriptRuns).toHaveLength(1);
    expect(lifecycle.worktrees.size).toBe(0);
    expect(operations.getState().tasks).toEqual([]);
  });

  it("skips teardown for worktrees that no longer exist", async () => {
    const { operations, lifecycle } = scripted(
      declared({ teardown: "make stop", teardownHash: "h" }),
      [task]
    );
    lifecycle.worktrees.clear();
    await operations.deleteTask("t1", { deleteWorktrees: true });
    expect(lifecycle.scriptRuns).toEqual([]);
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

  it("remembers the task's agent and skips the write when it is unchanged", async () => {
    const { operations, save } = fixture([task]);
    await operations.setTaskAgent("t1", "codex");
    expect(normalizeTasks(operations.getState().tasks)[0].agent).toBe("codex");
    save.mockClear();
    await operations.setTaskAgent("t1", "codex");
    expect(save).not.toHaveBeenCalled();
    await expect(operations.setTaskAgent("missing", "claude")).rejects.toMatchObject({
      code: "not_found",
    });
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

  it("does not use cached attachments to validate a ready action after an empty refresh", async () => {
    const workspace = operations.getState().workspaces[0];
    await taskLinearStore.refreshIssues(workspace, ["i1"]);
    fetchInfo.mockResolvedValue({});
    await expect(operations.markPrReady(task.id, pr)).rejects.toThrow("no longer linked");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("rejects a ready action if credentials change while validating the attachment", async () => {
    let resolve!: (info: Awaited<ReturnType<LinearService["fetchIssueLinearInfoBatch"]>>) => void;
    fetchInfo.mockImplementation(
      () =>
        new Promise((done) => {
          resolve = done;
        })
    );
    const request = operations.markPrReady(task.id, pr);
    await Promise.resolve();
    operations.getState().workspaces[0] = { ...workspace, linearApiKey: "new-key" };
    resolve({ i1: { status: null, prs: [pr] } });
    await expect(request).rejects.toThrow("no longer linked");
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
