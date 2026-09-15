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
import { persist } from "./store";
import { archiveTaskNote, ensureTaskNote } from "./notes";
import { closeTaskSessions } from "./taskSessions";

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

  createTask(input: CreateTaskInput, progress: (message: string) => void = () => {}) {
    return this.enqueue(async (): Promise<OperationResult<Task>> => {
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
      const members: TaskMember[] = [];
      for (const repo of repos) {
        const resolved = input.linearIssue
          ? {
              branchName: input.branchName.trim(),
              path: `${repo.worktreeBasePath}/${input.branchName.trim()}`,
            }
          : await invoke<{ branchName: string; path: string }>("resolve_manual_worktree", {
              repoPath: repo.localPath,
              worktreeBasePath: repo.worktreeBasePath,
              rawName: input.branchName,
            });
        members.push({
          repoId: repo.id,
          repoName: repo.name,
          localPath: repo.localPath,
          path: resolved.path,
          branchName: resolved.branchName,
        });
      }
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
      const created: TaskMember[] = [];
      const warnings: OperationWarning[] = [];
      for (const member of members) {
        progress(`Creating worktree for ${member.repoName}`);
        try {
          const added = await invoke<{ warning: string | null }>("git_worktree_add", {
            repoPath: member.localPath,
            worktreePath: member.path,
            branchName: member.branchName,
          });
          created.push(member);
          if (added.warning)
            warnings.push({
              stage: "git",
              repoId: member.repoId,
              message: "Worktree created using local refs; origin may be unavailable or stale.",
            });
        } catch {
          throw new OperationError(
            "create_failed",
            "Worktree creation failed; inspect the reported destinations before retrying.",
            { completedMembers: created, attemptedMember: member }
          );
        }
      }
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
      if (members.length > 1 && ["cursor", "vscode"].includes(editor)) {
        await this.optional(warnings, "workspace_file", undefined, async () => {
          task.workspaceFilePath = await invoke<string>("prepare_task_workspace", {
            workspaceName: workspace.name,
            branchName: task.branchName,
            folders: members.map((member) => member.path),
          });
        });
      }
      try {
        await this.write({ tasks: [...this.getState().tasks, task] });
      } catch {
        throw new OperationError(
          "persist_failed",
          "Worktrees exist but the task could not be saved.",
          { task }
        );
      }
      const setup: NonNullable<OperationResult<Task>["setup"]> = [];
      for (const member of members) {
        const steps: { stage: string; status: string }[] = [];
        setup.push({ repoId: member.repoId, steps });
        progress(`Preparing ${member.repoName}`);
        const copied = await this.optional(warnings, "config", member.repoId, () =>
          invoke("copy_local_configs", {
            sourceRepo: member.localPath,
            worktreePath: member.path,
            paths: [...EDITOR_CONFIG_PATHS[editor], ...ALWAYS_COPIED_CONFIG_PATHS],
          })
        );
        steps.push({ stage: "config", status: copied ? "completed" : "error" });
        for (const command of ["doppler_setup", "install_node_deps"]) {
          let status = "error";
          await this.optional(warnings, command, member.repoId, async () => {
            const result = await invoke<{ status: string }>(command, { worktreePath: member.path });
            status = result.status;
            if (result.status === "error" || result.status === "skipped_no_cli") throw new Error();
          });
          steps.push({ stage: command, status });
        }
      }
      const vault = this.getState().vault;
      await this.optional(warnings, "note", undefined, async () => {
        if (vault.enabled && vault.path && !(await ensureTaskNote(vault, workspace, task)))
          throw new Error();
      });
      progress("Task created and setup completed");
      return { data: task, warnings, setup };
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
      await this.archiveNotes(tasks, warnings);
      return { data: { id }, warnings };
    });
  }
}
