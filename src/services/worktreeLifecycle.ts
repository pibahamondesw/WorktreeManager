import { Channel, invoke } from "@tauri-apps/api/core";
import { Task, TaskMember, WorkspaceRepo } from "../types";
import { ResolvedScripts, resolveRepoScripts } from "./repoScripts";
import { DetectedSetupPlan, DetectedStep } from "./taskSetup";
import { runTaskScript, TaskScriptRun, terminalClose } from "./terminal";

export type WorktreeValidation = "valid" | "owned" | "invalid";

/** Native work behind a task's lifecycle; Operations owns ordering, persistence and recovery. */
export interface WorktreeLifecycle {
  validateRepos(repos: WorkspaceRepo[]): Promise<void>;
  resolveManualWorktree(
    repo: WorkspaceRepo,
    rawName: string,
    ownedPaths: string[]
  ): Promise<{ branchName: string; path: string }>;
  validateWorktrees(
    members: TaskMember[],
    basePaths: string[],
    ownedPaths: string[]
  ): Promise<WorktreeValidation>;
  addWorktree(member: TaskMember): Promise<{ warning: string | null }>;
  readScripts(worktreePath: string): Promise<ResolvedScripts>;
  /** Copies `paths` plus the `.worktreeinclude` selection, or `fallbackPaths` without one. */
  copyLocalConfigs(member: TaskMember, paths: string[], fallbackPaths: string[]): Promise<void>;
  prepareDetectedSetup(worktreePath: string): Promise<DetectedSetupPlan>;
  runDetectedSetup(
    plan: DetectedSetupPlan,
    onStep: (step: DetectedStep) => void
  ): Promise<DetectedStep[]>;
  /** Exit code of a repository script; null when it was stopped. */
  runScript(run: TaskScriptRun): Promise<number | null>;
  removeWorktree(member: TaskMember, force: boolean): Promise<void>;
  removeTaskWorkspace(task: Task): Promise<void>;
  /** Best-effort cleanup after removal; resolves with the cleanup stages that failed. */
  cleanupRemovedTasks(tasks: Task[]): Promise<string[]>;
}

const REMOVAL_CLEANUPS = ["cleanup_claude_json", "doppler_cleanup"];

export const tauriWorktreeLifecycle: WorktreeLifecycle = {
  validateRepos: (repos) => invoke("validate_workspace_repos", { repos }),
  resolveManualWorktree: (repo, rawName, ownedPaths) =>
    invoke("resolve_manual_worktree", {
      repoPath: repo.localPath,
      worktreeBasePath: repo.worktreeBasePath,
      rawName,
      ownedPaths,
    }),
  validateWorktrees: (members, basePaths, ownedPaths) =>
    invoke("validate_task_worktrees", { members, basePaths, ownedPaths }).then(
      () => "valid",
      (error) => (error === "owned" ? "owned" : "invalid")
    ),
  addWorktree: (member) =>
    invoke("git_worktree_add", {
      repoPath: member.localPath,
      worktreePath: member.path,
      branchName: member.branchName,
    }),
  readScripts: resolveRepoScripts,
  copyLocalConfigs: (member, paths, fallbackPaths) =>
    invoke("copy_local_configs", {
      sourceRepo: member.localPath,
      worktreePath: member.path,
      paths,
      fallbackPaths,
    }),
  prepareDetectedSetup: (worktreePath) => invoke("prepare_detected_setup", { worktreePath }),
  runDetectedSetup: (plan, onStep) => {
    const channel = new Channel<DetectedStep>();
    channel.onmessage = onStep;
    return invoke("run_detected_setup", { plan, onStep: channel });
  },
  runScript: runTaskScript,
  removeWorktree: (member, force) =>
    invoke("git_worktree_remove", {
      repoPath: member.localPath,
      worktreePath: member.path,
      force,
    }),
  removeTaskWorkspace: (task) =>
    invoke("remove_task_workspace", {
      taskId: task.id,
      legacyPath: task.workspaceFilePath ?? null,
    }),
  cleanupRemovedTasks: async (tasks) => {
    const failed = await invoke<string[]>("cleanup_removed_worktrees", {
      paths: tasks.flatMap((task) => task.members.map((member) => member.path)),
    }).catch(() => REMOVAL_CLEANUPS);
    await Promise.all(tasks.map((task) => terminalClose(task.id)));
    return failed;
  },
};
