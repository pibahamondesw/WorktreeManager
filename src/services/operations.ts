import { updateBinding, type Shortcut } from "../shortcuts/catalog";
import { syncNativeShortcuts, setShortcutOverrides } from "../shortcuts/runtime";
import {
  githubPrStatus,
  invalidateGithubRepo,
  prKey,
  refreshGithubRepo,
  setPrReadyPending,
} from "./github";
import { invoke } from "@tauri-apps/api/core";
import { v4 as uuid } from "uuid";
import {
  PullRequestInfo,
  AppState,
  Task,
  TaskMember,
  Workspace,
  EditorApp,
  EDITOR_CONFIG_PATHS,
  ALWAYS_COPIED_CONFIG_PATHS,
  VaultConfig,
} from "../types";
import { compareTaskPins } from "../utils";
import { LinearService } from "./linear";
import { persist } from "./store";
import { archiveTaskNote, ensureTaskNote, taskNoteFileName } from "./notes";
import { closeTaskSessions } from "./taskSessions";
import { VaultAgent, VAULT_AGENT_LABELS } from "./vault";
import {
  SETUP_STAGES,
  SetupStatus,
  DETECTED_STAGES,
  initializeTaskSetup,
  getTaskSetup,
  updateTaskSetup,
  updateSetupStep,
  updateRepoSetup,
  planRepoSetup,
  beginSetupRun,
  endSetupRun,
  clearTaskSetup,
} from "./taskSetup";
import {
  PhaseScript,
  ResolvedScripts,
  SCRIPT_SOURCE_LABELS,
  approveScripts,
  isScriptTrusted,
  phaseScript,
  resolveRepoScripts,
  scriptEnv,
  scriptSession,
} from "./repoScripts";
import { runTaskScript, scriptCancel, terminalClose } from "./terminal";

export type TaskReady = (task: Task) => Promise<boolean>;

export class OperationError extends Error {
  code: string;
  details?: unknown;
  constructor(code: string, message: string, details?: unknown) {
    super(message);
    this.code = code;
    this.details = details;
  }
}

export interface CreateTaskInput {
  workspaceId: string;
  branchName: string;
  repoIds?: string[];
  linearIssue?: {
    id: string;
    identifier: string;
    title: string;
    projectId?: string;
    projectName?: string;
  };
}

export interface DeleteOptions {
  deleteWorktrees: boolean;
  force?: boolean;
  /** Delete without running repository teardown scripts. */
  skipTeardown?: boolean;
  /** Approve the repository teardown scripts shown to the user before running them. */
  approveTeardown?: boolean;
}

/** How a repository's setup should run again after the task exists. */
export type SetupRetry = "retry" | "approve" | "detected";

const TEARDOWN_TIMEOUT_SECS = 300;

type SetupSteps = { stage: string; status: string }[];

type RepoScriptsResolution = { resolved: ResolvedScripts } | { error: string };

function repoScriptsResolution(path: string): Promise<RepoScriptsResolution> {
  return resolveRepoScripts(path).then(
    (resolved) => ({ resolved }),
    (error) => ({ error: typeof error === "string" ? error : "Could not read setup scripts." })
  );
}

function scriptStatus(code: number | null): SetupStatus {
  return code === 0 ? "completed" : code === null ? "cancelled" : "error";
}

export interface OperationWarning {
  stage: string;
  message: string;
  repoId?: string;
}

export interface OperationResult<T> {
  data: T;
  warnings: OperationWarning[];
  setup?: { repoId: string; steps: { stage: string; status: string }[] }[];
}

const EMPTY_SCRIPTS: ResolvedScripts = {
  present: true,
  source: null,
  setup: null,
  setupHash: null,
  teardown: null,
  teardownHash: null,
};

export function operationError(error: unknown): OperationError {
  return error instanceof OperationError
    ? error
    : new OperationError(
        "operation_failed",
        "The operation failed. Inspect the app and retry explicitly."
      );
}

export class Operations {
  private pending: Promise<unknown> = Promise.resolve();
  private readyRequests = new Map<string, Promise<void>>();
  readonly getState: () => AppState;
  private publish: (state: AppState) => void;
  private getEditor: () => EditorApp;
  private save: typeof persist;

  constructor(
    getState: () => AppState,
    publish: (state: AppState) => void,
    getEditor: () => EditorApp,
    save = persist
  ) {
    this.getState = getState;
    this.publish = publish;
    this.getEditor = getEditor;
    this.save = save;
  }

  private enqueue<T>(run: () => Promise<T>): Promise<T> {
    const result = this.pending.then(run);
    this.pending = result.catch(() => {});
    return result;
  }

  markPrReady(taskId: string, pr: PullRequestInfo): Promise<void> {
    const key = prKey(pr);
    const active = this.readyRequests.get(key);
    if (active) return active;
    setPrReadyPending(pr, true);
    const request = (async () => {
      const task = this.getState().tasks.find((task) => task.id === taskId);
      const workspace = this.getState().workspaces.find(
        (workspace) => workspace.id === task?.workspaceId
      );
      if (!task?.linearIssueId || !workspace?.linearApiKey) {
        throw new OperationError(
          "invalid_input",
          "The pull request is no longer linked to this task."
        );
      }
      const info = await new LinearService(workspace.linearApiKey).fetchIssueLinearInfoBatch([
        task.linearIssueId,
      ]);
      const linked = info[task.linearIssueId]?.prs ?? [];
      if (
        !linked.some((candidate) => prKey(candidate) === key) ||
        this.getState().tasks.find((current) => current.id === taskId)?.linearIssueId !==
          task.linearIssueId
      ) {
        throw new OperationError(
          "invalid_input",
          "The pull request is no longer linked to this task."
        );
      }
      const status = githubPrStatus(pr);
      if (!status || status.state !== "open" || !status.isDraft) {
        throw new OperationError(
          "invalid_input",
          "Refresh the pull request before marking it ready."
        );
      }
      await invoke("github_pr_ready", { repoSlug: pr.repoSlug.toLowerCase(), prNumber: pr.number });
      invalidateGithubRepo(pr.repoSlug, pr);
      await refreshGithubRepo(pr.repoSlug);
    })().finally(() => {
      this.readyRequests.delete(key);
      setPrReadyPending(pr, false);
    });
    this.readyRequests.set(key, request);
    return request;
  }

  private async write(patch: Partial<AppState>) {
    try {
      await this.save(Object.entries(patch));
    } catch {
      throw new OperationError(
        "persist_failed",
        "Could not save changes; the app record was retained."
      );
    }
    this.publish({ ...this.getState(), ...patch });
  }

  change(build: (state: AppState) => Partial<AppState>) {
    return this.enqueue(() => this.write(build(this.getState())));
  }

  updateShortcut(id: string, values: Shortcut[] | null, reassign = false) {
    return this.enqueue(async () => {
      const previous = this.getState().shortcutOverrides ?? {};
      const next = updateBinding(previous, id, values, reassign);
      if (JSON.stringify(previous) === JSON.stringify(next)) return;
      await syncNativeShortcuts(next);
      try {
        await this.write({ shortcutOverrides: next });
      } catch (error) {
        await syncNativeShortcuts(previous);
        throw error;
      }
      setShortcutOverrides(next);
    });
  }

  resetShortcuts() {
    return this.enqueue(async () => {
      const previous = this.getState().shortcutOverrides ?? {};
      await syncNativeShortcuts({});
      try {
        await this.write({ shortcutOverrides: {} });
      } catch (error) {
        await syncNativeShortcuts(previous);
        throw error;
      }
      setShortcutOverrides({});
    });
  }

  updateVault(vault: VaultConfig) {
    return this.enqueue(async () => {
      if (vault.enabled && !vault.path?.startsWith("/")) {
        throw new OperationError("invalid_input", "An enabled vault needs an absolute path.");
      }
      // Save first: global agent setup is optional and must not undo a usable vault.
      await this.write({ vault });
      await this.syncVaultAgents(vault, false);
    });
  }

  repairVaultAgents(agent?: VaultAgent) {
    return this.enqueue(() =>
      this.syncVaultAgents(this.getState().vault, true, agent ? [agent] : undefined)
    );
  }

  private async syncVaultAgents(
    vault: VaultConfig,
    repair: boolean,
    agents: VaultAgent[] = ["codex", "claude"]
  ) {
    if (vault.enabled && !vault.path?.startsWith("/")) {
      throw new OperationError(
        "invalid_input",
        "Configure an absolute vault path in settings before repairing agent setup."
      );
    }
    const failures: string[] = [];
    // Attempt both agents even when one fails, and expose every partial result.
    for (const agent of agents) {
      try {
        await invoke("sync_vault_agent", {
          agent,
          vaultPath: vault.enabled ? vault.path : null,
          repair,
        });
      } catch (error) {
        failures.push(`${VAULT_AGENT_LABELS[agent]}: ${String(error)}`);
      }
    }
    if (failures.length) {
      throw new OperationError(
        "vault_agent_setup_failed",
        `Vault settings were saved, but agent ${vault.enabled ? "setup" : "cleanup"} failed: ${failures.join("; ")}. Retry from vault settings${vault.enabled ? " or Dependencies" : ""}.`
      );
    }
  }

  refresh(load: (getState: () => AppState) => Promise<AppState>) {
    return this.enqueue(async () => {
      const state = await load(this.getState);
      this.publish(state);
      return state;
    });
  }

  workspace(id: string) {
    const workspace = this.getState().workspaces.find((item) => item.id === id);
    if (!workspace) throw new OperationError("not_found", "Workspace not found.");
    return workspace;
  }

  task(id: string) {
    const task = this.getState().tasks.find((item) => item.id === id);
    if (!task) throw new OperationError("not_found", "Task not found.");
    return task;
  }

  setTaskPinned(id: string, pinned: boolean) {
    return this.change((state) => {
      const task = this.task(id);
      if ((task.pinOrder !== undefined) === pinned) return {};
      const pins = state.tasks.filter(
        (item) => item.workspaceId === task.workspaceId && item.pinOrder !== undefined
      );
      const pinOrder = pinned ? Math.max(-1, ...pins.map((item) => item.pinOrder!)) + 1 : undefined;
      return { tasks: state.tasks.map((item) => (item.id === id ? { ...item, pinOrder } : item)) };
    });
  }

  reorderPinnedTasks(id: string, targetId: string) {
    return this.change((state) => {
      const task = this.task(id);
      const target = this.task(targetId);
      if (
        task.workspaceId !== target.workspaceId ||
        task.pinOrder === undefined ||
        target.pinOrder === undefined
      ) {
        throw new OperationError(
          "invalid_params",
          "Only pinned tasks in the same workspace can be reordered."
        );
      }
      const pins = state.tasks
        .filter((item) => item.workspaceId === task.workspaceId && item.pinOrder !== undefined)
        .sort(compareTaskPins);
      const from = pins.findIndex((item) => item.id === id);
      const to = pins.findIndex((item) => item.id === targetId);
      pins.splice(to, 0, ...pins.splice(from, 1));
      const positions = new Map(pins.map((item, index) => [item.id, index]));
      return {
        tasks: state.tasks.map((item) =>
          positions.has(item.id) ? { ...item, pinOrder: positions.get(item.id)! } : item
        ),
      };
    });
  }

  private async validateWorkspace(workspace: Workspace) {
    if (!workspace.name.trim() || !workspace.repos.length) {
      throw new OperationError(
        "invalid_params",
        "A workspace needs a name and at least one repository."
      );
    }
    if (new Set(workspace.repos.map((repo) => repo.id)).size !== workspace.repos.length) {
      throw new OperationError("invalid_params", "Repository IDs must be unique.");
    }
    await invoke("validate_workspace_repos", { repos: workspace.repos }).catch(() => {
      throw new OperationError(
        "invalid_repositories",
        "Repositories must be distinct Git repositories with absolute, non-overlapping worktree directories."
      );
    });
  }

  addWorkspace(workspace: Workspace, select = true) {
    return this.enqueue(async () => {
      await this.validateWorkspace(workspace);
      if (this.getState().workspaces.some((item) => item.id === workspace.id)) {
        throw new OperationError("conflict", "Workspace already exists.");
      }
      await this.write({
        workspaces: [...this.getState().workspaces, workspace],
        ...(select ? { selectedWorkspaceId: workspace.id } : {}),
      });
      return workspace;
    });
  }

  updateWorkspace(
    id: string,
    patch: Partial<Pick<Workspace, "name" | "repos" | "linearApiKey" | "linearOrgUrlKey">>
  ) {
    return this.enqueue(async () => {
      const workspace = { ...this.workspace(id), ...patch };
      await this.validateWorkspace(workspace);
      await this.write({
        workspaces: this.getState().workspaces.map((item) => (item.id === id ? workspace : item)),
      });
      return workspace;
    });
  }

  async createTask(
    input: CreateTaskInput,
    progress: (message: string) => void = () => {},
    onReady?: TaskReady
  ): Promise<OperationResult<Task>> {
    const { task, workspace, editor, warnings, vault } = await this.enqueue(async () => {
      const workspace = this.workspace(input.workspaceId);
      const editor = this.getEditor();
      const repos = input.repoIds
        ? workspace.repos.filter((repo) => input.repoIds!.includes(repo.id))
        : workspace.repos;
      if (
        !input.branchName.trim() ||
        !repos.length ||
        (input.repoIds && repos.length !== new Set(input.repoIds).size)
      ) {
        throw new OperationError(
          "invalid_params",
          "Provide a branch and at least one repository belonging to the workspace."
        );
      }
      await this.validateWorkspace({ ...workspace, repos });
      const ownedPaths = this.getState().tasks.flatMap((task) =>
        task.members.map((member) => member.path)
      );
      const members: TaskMember[] = await Promise.all(
        repos.map(async (repo) => {
          const resolved = input.linearIssue
            ? {
                branchName: input.branchName.trim(),
                path: `${repo.worktreeBasePath}/${input.branchName.trim()}`,
              }
            : await invoke<{ branchName: string; path: string }>("resolve_manual_worktree", {
                repoPath: repo.localPath,
                worktreeBasePath: repo.worktreeBasePath,
                rawName: input.branchName,
                ownedPaths,
              });
          return {
            repoId: repo.id,
            repoName: repo.name,
            localPath: repo.localPath,
            path: resolved.path,
            branchName: resolved.branchName,
          };
        })
      );
      if (
        this.getState().tasks.some((task) =>
          task.members.some((member) => members.some((next) => next.path === member.path))
        )
      ) {
        throw new OperationError("conflict", "A task already owns one of these worktrees.");
      }
      await invoke("validate_task_worktrees", {
        members,
        basePaths: repos.map((repo) => repo.worktreeBasePath),
        ownedPaths: this.getState().tasks.flatMap((task) =>
          task.members.map((member) => member.path)
        ),
      }).catch(() => {
        throw new OperationError(
          "invalid_worktrees",
          "Check branch names, destination paths, and existing worktree ownership."
        );
      });
      const warnings: OperationWarning[] = [];
      let completed = 0;
      progress(`Creating worktrees… 0/${members.length} completed`);
      const results = await Promise.allSettled(
        members.map(async (member) => {
          const added = await invoke<{ warning: string | null }>("git_worktree_add", {
            repoPath: member.localPath,
            worktreePath: member.path,
            branchName: member.branchName,
          });
          completed += 1;
          progress(`Creating worktrees… ${completed}/${members.length} completed`);
          return added;
        })
      );
      const failedMembers = members.filter((_, index) => results[index].status === "rejected");
      if (failedMembers.length) {
        throw new OperationError(
          "create_failed",
          "Some worktrees could not be created. Existing worktrees were retained.",
          {
            completedMembers: members.filter((_, index) => results[index].status === "fulfilled"),
            failedMembers,
            attemptedMember: failedMembers[0],
          }
        );
      }
      results.forEach((result, index) => {
        if (result.status === "fulfilled" && result.value.warning)
          warnings.push({
            stage: "git",
            repoId: members[index].repoId,
            message: "Worktree created using local refs; origin may be unavailable or stale.",
          });
      });
      const taskId = uuid();
      const task: Task = {
        id: taskId,
        noteFolder: taskId,
        workspaceId: workspace.id,
        branchName: members[0].branchName,
        members,
        createdAt: new Date().toISOString(),
        workspaceFilePath: null,
        ...(input.linearIssue
          ? {
              linearIssueId: input.linearIssue.id,
              linearIssueIdentifier: input.linearIssue.identifier,
              linearIssueTitle: input.linearIssue.title,
              linearProjectId: input.linearIssue.projectId,
              linearProjectName: input.linearIssue.projectName,
            }
          : {}),
      };
      try {
        await this.write({ tasks: [...this.getState().tasks, task] });
      } catch {
        throw new OperationError(
          "persist_failed",
          "Worktrees exist but the task could not be saved.",
          { task }
        );
      }
      initializeTaskSetup(task, warnings);
      return { task, workspace, editor, warnings, vault: this.getState().vault };
    });
    const setup: NonNullable<OperationResult<Task>["setup"]> = task.members.map((member) => ({
      repoId: member.repoId,
      steps: [],
    }));
    const resolutions = new Map<string, RepoScriptsResolution>();
    try {
      progress("Copying local configuration…");
      await Promise.all([
        ...task.members.map(async (member, index) => {
          const resolution = await repoScriptsResolution(member.path);
          resolutions.set(member.repoId, resolution);
          updateSetupStep(task.id, member.repoId, "config", "running", warnings);
          const copied = await this.optional(warnings, "config", member.repoId, async () => {
            const included = await invoke<string[] | null>("worktree_include_paths", {
              repoPath: member.localPath,
            });
            await invoke("copy_local_configs", {
              sourceRepo: member.localPath,
              worktreePath: member.path,
              paths: [...EDITOR_CONFIG_PATHS[editor], ...(included ?? ALWAYS_COPIED_CONFIG_PATHS)],
            });
          });
          setup[index].steps.push({ stage: "config", status: copied ? "completed" : "error" });
          updateSetupStep(
            task.id,
            member.repoId,
            "config",
            copied ? "completed" : "error",
            warnings
          );
          if (!this.setupScript(member.repoId, workspace, resolution))
            await invoke("prepare_python_env", { worktreePath: member.path }).catch(
              () => undefined
            );
        }),
        this.optional(warnings, "note", undefined, async () => {
          if (vault.enabled && vault.path && !(await ensureTaskNote(vault, workspace, task)))
            throw new Error();
        }),
        this.optional(warnings, "workspace_file", undefined, async () => {
          if (task.members.length <= 1 || !["cursor", "vscode"].includes(editor)) return;
          const workspaceFilePath = await invoke<string>("prepare_task_workspace", {
            workspaceName: workspace.name,
            branchName: task.branchName,
            folders: task.members.map((member) => member.path),
          });
          await this.change((state) => ({
            tasks: state.tasks.map((item) =>
              item.id === task.id ? { ...item, workspaceFilePath } : item
            ),
          }));
        }),
      ]);
      if (onReady) {
        progress("Opening workspace…");
        await this.optional(warnings, "open", undefined, async () => {
          if (!(await onReady(this.task(task.id)))) throw new Error();
          updateTaskSetup(task.id, { ...getTaskSetup(task.id)!, opened: true });
        });
      }
      await Promise.all(
        task.members.map((member, index) =>
          this.runRepoSetup(
            task,
            member,
            resolutions.get(member.repoId)!,
            warnings,
            setup[index].steps,
            progress
          )
        )
      );
      warnings.sort(
        (a, b) =>
          task.members.findIndex((m) => m.repoId === a.repoId) -
          task.members.findIndex((m) => m.repoId === b.repoId)
      );
      progress(warnings.length ? "Setup completed with warnings" : "Setup completed");
      return { data: this.task(task.id), warnings, setup };
    } finally {
      endSetupRun(task.id, warnings);
    }
  }

  /** The setup script that applies to a repository, or a resolution error for explicit config. */
  private setupScript(
    repoId: string,
    workspace: Workspace | undefined,
    resolution: RepoScriptsResolution
  ): PhaseScript | { error: string } | null {
    const repo = workspace?.repos.find((item) => item.id === repoId);
    if ("error" in resolution)
      return repo?.scripts?.setup ? phaseScript(repo, EMPTY_SCRIPTS, "setup") : resolution;
    return phaseScript(repo, resolution.resolved, "setup");
  }

  /**
   * Prepare one repository after its worktree exists: its declared setup script when there is
   * one (once approved), otherwise the detected Doppler, Node and Python steps.
   */
  private async runRepoSetup(
    task: Task,
    member: TaskMember,
    resolution: RepoScriptsResolution,
    warnings: OperationWarning[],
    steps: SetupSteps,
    progress: (message: string) => void = () => undefined,
    choice: SetupRetry = "retry"
  ) {
    const workspace = this.getState().workspaces.find((item) => item.id === task.workspaceId);
    const script =
      choice === "detected" ? null : this.setupScript(member.repoId, workspace, resolution);
    if (script && "error" in script) {
      planRepoSetup(task.id, member.repoId, ["script"], "worktreemanager");
      warnings.push({ stage: "script", repoId: member.repoId, message: script.error });
      steps.push({ stage: "script", status: "error" });
      updateSetupStep(task.id, member.repoId, "script", "error", warnings, script.error);
      return;
    }
    if (!script) {
      planRepoSetup(task.id, member.repoId, DETECTED_STAGES);
      await this.runDetectedSetup(task, member, warnings, steps, progress);
      return;
    }
    planRepoSetup(task.id, member.repoId, ["script"], script.source);
    if (choice === "approve" && script.hash) await approveScripts(member.localPath, [script.hash]);
    if (!(await isScriptTrusted(member.localPath, script))) {
      const message = `Review the setup script from ${SCRIPT_SOURCE_LABELS[script.source]} before it runs.`;
      warnings.push({ stage: "script", repoId: member.repoId, message });
      steps.push({ stage: "script", status: "needs_approval" });
      updateSetupStep(task.id, member.repoId, "script", "needs_approval", warnings);
      updateRepoSetup(task.id, member.repoId, (repo) => ({ ...repo, approval: script }));
      return;
    }
    progress(`${member.repoName} · ${SETUP_STAGES.script}…`);
    const session = scriptSession("setup", member.repoId);
    updateSetupStep(task.id, member.repoId, "script", "running", warnings);
    const code = await runTaskScript({
      taskId: task.id,
      session,
      script: script.script,
      cwd: member.path,
      env: scriptEnv(task, member, workspace),
    }).catch(() => undefined);
    const status = code === undefined ? "error" : scriptStatus(code);
    const message =
      status === "completed"
        ? undefined
        : status === "cancelled"
          ? "Setup script was stopped."
          : code === undefined
            ? "Setup script could not start."
            : `Setup script exited with code ${code}.`;
    if (message)
      warnings.push({
        stage: "script",
        repoId: member.repoId,
        message: `${message} Open its output to inspect and retry.`,
      });
    steps.push({ stage: "script", status });
    updateSetupStep(task.id, member.repoId, "script", status, warnings, message);
  }

  private async runDetectedSetup(
    task: Task,
    member: TaskMember,
    warnings: OperationWarning[],
    steps: SetupSteps,
    progress: (message: string) => void
  ) {
    for (const stage of DETECTED_STAGES) {
      progress(`${member.repoName} · ${SETUP_STAGES[stage]}…`);
      updateSetupStep(task.id, member.repoId, stage, "running", warnings);
      let status = "error";
      let message: string | undefined;
      await this.optional(warnings, stage, member.repoId, async () => {
        const result = await invoke<{ status: string; message?: string }>(stage, {
          worktreePath: member.path,
        });
        status = result.status;
        if (status === "error" || status === "skipped_no_cli") {
          message = result.message;
          throw new Error();
        }
      });
      steps.push({ stage, status });
      updateSetupStep(
        task.id,
        member.repoId,
        stage,
        status === "error" || status === "skipped_no_cli"
          ? "error"
          : status.startsWith("skipped_")
            ? "skipped"
            : "completed",
        warnings,
        message
      );
    }
  }

  /**
   * Run one repository's setup again: re-read its scripts (so edits apply), approve the shown
   * script first, or fall back to detected setup.
   */
  async rerunRepoSetup(
    taskId: string,
    repoId: string,
    choice: SetupRetry = "retry"
  ): Promise<OperationResult<{ repoId: string; steps: SetupSteps }>> {
    const task = this.task(taskId);
    const member = task.members.find((item) => item.repoId === repoId);
    if (!member) throw new OperationError("not_found", "Repository not found in this task.");
    const running = getTaskSetup(taskId)?.repos.find((repo) => repo.repoId === repoId);
    if (running?.steps.some((step) => step.status === "running"))
      throw new OperationError("setup_active", "This repository's setup is still running.");
    beginSetupRun(task);
    const warnings: OperationWarning[] = [];
    const steps: SetupSteps = [];
    try {
      const resolution = await repoScriptsResolution(member.path);
      if ("resolved" in resolution && !resolution.resolved.present)
        throw new OperationError("not_found", "The worktree no longer exists.");
      await this.runRepoSetup(task, member, resolution, warnings, steps, undefined, choice);
      return { data: { repoId, steps }, warnings };
    } finally {
      const previous = getTaskSetup(taskId)?.warnings ?? [];
      endSetupRun(taskId, [...previous.filter((item) => item.repoId !== repoId), ...warnings]);
    }
  }

  /** Stop a running setup script; its output stays available. */
  cancelRepoSetup(taskId: string, repoId: string) {
    return scriptCancel(taskId, scriptSession("setup", repoId));
  }

  /**
   * Run each repository's teardown script before its worktree is removed. Missing worktrees are
   * skipped. Repository scripts need approval: unapproved ones stop the deletion so the user can
   * review them, and any failure retains the task.
   */
  private async runTeardowns(tasks: Task[], options: DeleteOptions) {
    const pending: { task: Task; member: TaskMember; script: PhaseScript }[] = [];
    for (const task of tasks) {
      const workspace = this.getState().workspaces.find((item) => item.id === task.workspaceId);
      for (const member of task.members) {
        const repo = workspace?.repos.find((item) => item.id === member.repoId);
        const resolution = await repoScriptsResolution(member.path);
        if ("resolved" in resolution && !resolution.resolved.present) continue;
        const script = phaseScript(
          repo,
          "resolved" in resolution ? resolution.resolved : EMPTY_SCRIPTS,
          "teardown"
        );
        if (script) pending.push({ task, member, script });
        else if ("error" in resolution)
          throw new OperationError("teardown_failed", resolution.error, {
            repoName: member.repoName,
          });
      }
    }
    if (!pending.length) return;
    const untrusted = [];
    for (const item of pending)
      if (!(await isScriptTrusted(item.member.localPath, item.script))) untrusted.push(item);
    if (untrusted.length && !options.approveTeardown)
      throw new OperationError(
        "teardown_needs_approval",
        "Review the repository teardown scripts before they run, or delete without them.",
        {
          scripts: untrusted.map(({ member, script }) => ({
            repoName: member.repoName,
            source: script.source,
            script: script.script,
          })),
        }
      );
    for (const { member, script } of untrusted)
      await approveScripts(member.localPath, [script.hash!]);
    const failed: string[] = [];
    for (const task of tasks) {
      const taskScripts = pending.filter((item) => item.task === task);
      if (!taskScripts.length) continue;
      beginSetupRun(task);
      const warnings = getTaskSetup(task.id)?.warnings ?? [];
      try {
        await Promise.all(
          taskScripts.map(async ({ member, script }) => {
            updateSetupStep(task.id, member.repoId, "teardown", "running", warnings);
            const workspace = this.getState().workspaces.find(
              (item) => item.id === task.workspaceId
            );
            const code = await runTaskScript({
              taskId: task.id,
              session: scriptSession("teardown", member.repoId),
              script: script.script,
              cwd: member.path,
              env: scriptEnv(task, member, workspace),
              timeoutSecs: TEARDOWN_TIMEOUT_SECS,
            }).catch(() => undefined);
            const status = code === undefined ? "error" : scriptStatus(code);
            if (status !== "completed") failed.push(member.repoName);
            updateSetupStep(
              task.id,
              member.repoId,
              "teardown",
              status === "cancelled" ? "error" : status,
              warnings,
              status === "completed"
                ? undefined
                : code == null
                  ? "Teardown script did not finish."
                  : `Teardown script exited with code ${code}.`
            );
          })
        );
      } finally {
        endSetupRun(task.id);
      }
    }
    if (failed.length)
      throw new OperationError(
        "teardown_failed",
        `Teardown failed for ${failed.join(", ")}. The task was retained; inspect the output, or delete without teardown.`,
        { repoNames: failed }
      );
  }

  async refreshTaskProjects(workspaceId: string) {
    const workspace = this.getState().workspaces.find((item) => item.id === workspaceId);
    if (!workspace?.linearApiKey) return;
    const tasks = this.getState().tasks.filter((task) => task.workspaceId === workspaceId);
    const issueIds = tasks.flatMap((task) => (task.linearIssueId ? [task.linearIssueId] : []));
    const info = await new LinearService(workspace.linearApiKey).fetchIssueLinearInfoBatch(
      issueIds
    );
    return this.enqueue(async () => {
      if (
        this.getState().workspaces.find((item) => item.id === workspaceId)?.linearApiKey !==
        workspace.linearApiKey
      )
        return;
      let changed = false;
      const updated = this.getState().tasks.map((task) => {
        const project =
          task.workspaceId === workspaceId && task.linearIssueId
            ? info[task.linearIssueId]?.project
            : undefined;
        if (
          project === undefined ||
          (task.linearProjectId === project?.id && task.linearProjectName === project?.name)
        )
          return task;
        changed = true;
        return { ...task, linearProjectId: project?.id, linearProjectName: project?.name };
      });
      if (changed) await this.write({ tasks: updated });
    });
  }

  linkTaskIssue(id: string, identifier: string) {
    return this.enqueue(async () => {
      const task = this.task(id);
      if (!identifier.trim())
        throw new OperationError("invalid_params", "Provide a Linear issue ID.");
      if (task.linearIssueId) {
        if (
          [task.linearIssueId, task.linearIssueIdentifier?.toLowerCase()].includes(
            identifier.trim().toLowerCase()
          )
        )
          return task;
        throw new OperationError("conflict", "This task already has a Linear issue.");
      }
      const workspace = this.workspace(task.workspaceId);
      if (!workspace.linearApiKey)
        throw new OperationError(
          "linear_not_configured",
          "Configure Linear in this workspace first."
        );
      const issue = await new LinearService(workspace.linearApiKey)
        .getIssue(identifier.trim())
        .catch(() => {
          throw new OperationError(
            "linear_issue_unavailable",
            "Could not load the Linear issue. Check the issue ID, workspace credentials, and connection."
          );
        });
      const updated = {
        ...task,
        noteFileName: taskNoteFileName(task),
        linearIssueId: issue.id,
        linearIssueIdentifier: issue.identifier,
        linearIssueTitle: issue.title,
        linearProjectId: issue.projectId,
        linearProjectName: issue.projectName,
      };
      await this.write({
        tasks: this.getState().tasks.map((item) => (item.id === id ? updated : item)),
      });
      return updated;
    });
  }

  renameTask(id: string, title: string) {
    return this.enqueue(async () => {
      const task = this.task(id);
      const trimmed = title.trim();
      if (!trimmed) throw new OperationError("invalid_params", "Provide a title.");
      if (!task.linearIssueId)
        throw new OperationError("invalid_params", "Only tasks linked to Linear can be renamed.");
      if (trimmed === task.linearIssueTitle) return task;
      const workspace = this.workspace(task.workspaceId);
      if (!workspace.linearApiKey)
        throw new OperationError(
          "linear_not_configured",
          "Configure Linear in this workspace first."
        );
      await new LinearService(workspace.linearApiKey)
        .updateIssueTitle(task.linearIssueId, trimmed)
        .catch(() => {
          throw new OperationError(
            "linear_update_failed",
            "Could not rename the Linear issue. Check the workspace credentials and connection."
          );
        });
      const updated = { ...this.task(id), linearIssueTitle: trimmed };
      await this.write({
        tasks: this.getState().tasks.map((item) => (item.id === id ? updated : item)),
      });
      return updated;
    });
  }

  private async optional(
    warnings: OperationWarning[],
    stage: string,
    repoId: string | undefined,
    run: () => Promise<unknown>
  ) {
    try {
      await run();
      return true;
    } catch {
      warnings.push({
        stage,
        ...(repoId ? { repoId } : {}),
        message: `${stage} did not complete; retry this setup or cleanup step manually.`,
      });
      return false;
    }
  }

  private async removeResources(
    tasks: Task[],
    options: DeleteOptions,
    warnings: OperationWarning[]
  ) {
    if (tasks.some((task) => getTaskSetup(task.id)?.active)) {
      throw new OperationError(
        "setup_active",
        "Setup is still running. Wait for it to finish before deleting this task or workspace."
      );
    }
    try {
      await Promise.all(tasks.map((task) => closeTaskSessions(task.id)));
    } catch {
      throw new OperationError(
        "sessions_active",
        "Could not close task sessions; records were retained."
      );
    }
    if (!options.deleteWorktrees || !tasks.length) return;
    if (!options.skipTeardown) await this.runTeardowns(tasks, options);
    const removed: TaskMember[] = [];
    const failed: TaskMember[] = [];
    for (const task of tasks) {
      for (const member of task.members) {
        try {
          await invoke("git_worktree_remove", {
            repoPath: member.localPath,
            worktreePath: member.path,
            force: options.force ?? false,
          });
          removed.push(member);
        } catch {
          failed.push(member);
        }
      }
    }
    if (failed.length)
      throw new OperationError(
        "delete_failed",
        "Some worktrees could not be removed. Records were retained; inspect local changes before retrying with --force.",
        { removed, failed }
      );
    for (const task of tasks) {
      if (task.workspaceFilePath)
        await this.optional(warnings, "workspace_file", undefined, () =>
          invoke("delete_workspace_file", { path: task.workspaceFilePath })
        );
    }
    const paths = tasks.flatMap((task) => task.members.map((member) => member.path));
    for (const command of ["cleanup_claude_json", "doppler_cleanup"]) {
      await this.optional(warnings, command, undefined, () => invoke(command, { paths }));
    }
    await Promise.all(tasks.map((task) => terminalClose(task.id)));
  }

  private async archiveNotes(tasks: Task[], warnings: OperationWarning[]) {
    for (const task of tasks)
      await this.optional(warnings, "archive_note", undefined, () =>
        archiveTaskNote(this.getState().vault, task, true)
      );
  }

  deleteTask(id: string, options: DeleteOptions): Promise<OperationResult<{ id: string }>> {
    return this.enqueue(async () => {
      const task = this.task(id);
      const warnings: OperationWarning[] = [];
      await this.removeResources([task], options, warnings);
      await this.write({ tasks: this.getState().tasks.filter((item) => item.id !== id) });
      clearTaskSetup(id);
      await this.archiveNotes([task], warnings);
      return { data: { id }, warnings };
    });
  }

  deleteWorkspace(id: string, options?: DeleteOptions): Promise<OperationResult<{ id: string }>> {
    return this.enqueue(async () => {
      this.workspace(id);
      const tasks = this.getState().tasks.filter((task) => task.workspaceId === id);
      if (tasks.length && !options)
        throw new OperationError(
          "invalid_params",
          "Choose --keep-worktrees or --delete-worktrees."
        );
      const warnings: OperationWarning[] = [];
      await this.removeResources(tasks, options ?? { deleteWorktrees: false }, warnings);
      const state = this.getState();
      const workspaces = state.workspaces.filter((workspace) => workspace.id !== id);
      await this.write({
        workspaces,
        tasks: state.tasks.filter((task) => task.workspaceId !== id),
        selectedWorkspaceId:
          state.selectedWorkspaceId === id
            ? (workspaces[0]?.id ?? null)
            : state.selectedWorkspaceId,
      });
      tasks.forEach((task) => clearTaskSetup(task.id));
      await this.archiveNotes(tasks, warnings);
      return { data: { id }, warnings };
    });
  }
}
