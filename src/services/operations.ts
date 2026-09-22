import { invoke } from "@tauri-apps/api/core";
import { v4 as uuid } from "uuid";
import {
  AppState,
  Task,
  TaskMember,
  Workspace,
  EditorApp,
  EDITOR_CONFIG_PATHS,
  ALWAYS_COPIED_CONFIG_PATHS,
} from "../types";
import { LinearService } from "./linear";
import { persist } from "./store";
import { archiveTaskNote, ensureTaskNote, taskNoteFileName } from "./notes";
import { closeTaskSessions } from "./taskSessions";
import {
  SETUP_STAGES,
  SetupStage,
  initializeTaskSetup,
  getTaskSetup,
  updateTaskSetup,
  updateSetupStep,
  clearTaskSetup,
} from "./taskSetup";

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
  linearIssue?: { id: string; identifier: string; title: string };
}

export interface DeleteOptions {
  deleteWorktrees: boolean;
  force?: boolean;
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
    try {
      progress("Copying local configuration…");
      await Promise.all([
        ...task.members.map(async (member, index) => {
          updateSetupStep(task.id, member.repoId, "config", "running", warnings);
          const copied = await this.optional(warnings, "config", member.repoId, () =>
            invoke("copy_local_configs", {
              sourceRepo: member.localPath,
              worktreePath: member.path,
              paths: [...EDITOR_CONFIG_PATHS[editor], ...ALWAYS_COPIED_CONFIG_PATHS],
            })
          );
          setup[index].steps.push({ stage: "config", status: copied ? "completed" : "error" });
          updateSetupStep(
            task.id,
            member.repoId,
            "config",
            copied ? "completed" : "error",
            warnings
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
        task.members.map(async (member, index) => {
          for (const stage of [
            "doppler_setup",
            "install_node_deps",
            "install_python_deps",
          ] as SetupStage[]) {
            progress(`${member.repoName} · ${SETUP_STAGES[stage]}…`);
            updateSetupStep(task.id, member.repoId, stage, "running", warnings);
            let status = "error";
            await this.optional(warnings, stage, member.repoId, async () => {
              const result = await invoke<{ status: string }>(stage, { worktreePath: member.path });
              status = result.status;
              if (status === "error" || status === "skipped_no_cli") throw new Error();
            });
            setup[index].steps.push({ stage, status });
            updateSetupStep(
              task.id,
              member.repoId,
              stage,
              status === "error" || status === "skipped_no_cli"
                ? "error"
                : status.startsWith("skipped_")
                  ? "skipped"
                  : "completed",
              warnings
            );
          }
        })
      );
      warnings.sort(
        (a, b) =>
          task.members.findIndex((m) => m.repoId === a.repoId) -
          task.members.findIndex((m) => m.repoId === b.repoId)
      );
      progress(warnings.length ? "Setup completed with warnings" : "Setup completed");
      return { data: this.task(task.id), warnings, setup };
    } finally {
      updateTaskSetup(task.id, {
        ...getTaskSetup(task.id)!,
        active: false,
        warnings: [...warnings],
      });
    }
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
      };
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
