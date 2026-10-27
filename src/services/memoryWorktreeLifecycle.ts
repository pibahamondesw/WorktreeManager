import { Task, TaskMember, WorkspaceRepo } from "../types";
import { ResolvedScripts } from "./repoScripts";
import { DetectedSetupPlan, DetectedStep } from "./taskSetup";
import { TaskScriptRun } from "./terminal";
import { WorktreeLifecycle, WorktreeValidation } from "./worktreeLifecycle";

type Step = keyof WorktreeLifecycle;
type Target = (target: string) => boolean;

const NO_SCRIPTS: ResolvedScripts = {
  present: true,
  source: null,
  setup: null,
  setupHash: null,
  teardown: null,
  teardownHash: null,
};

const anyTarget: Target = () => true;

/** In-memory lifecycle: worktrees, copied configs and script runs are plain state to inspect. */
export class MemoryWorktreeLifecycle implements WorktreeLifecycle {
  readonly worktrees = new Map<string, TaskMember>();
  readonly copiedConfigs = new Map<string, string[]>();
  /** `.worktreeinclude` selections by repository path. */
  readonly includes = new Map<string, string[]>();
  /** Declared scripts by repository path; a string is a config error. */
  readonly scripts = new Map<string, ResolvedScripts | string>();
  readonly scriptRuns: TaskScriptRun[] = [];
  readonly removedTaskWorkspaces: Task[] = [];
  readonly cleanedTasks: Task[] = [];
  readonly failingCleanups: string[] = [];
  readonly calls: { step: Step; target: string }[] = [];
  detectedSteps: (worktreePath: string) => DetectedStep[] = () => [];
  exitCode: (run: TaskScriptRun) => number | null = () => 0;
  private failures = new Map<Step, Target>();
  private holds = new Map<Step, { when: Target; released: Promise<void> }>();

  seed(tasks: Task[]) {
    for (const member of tasks.flatMap((task) => task.members))
      this.worktrees.set(member.path, member);
    return this;
  }

  fail(step: Step, when = anyTarget) {
    this.failures.set(step, when);
  }

  succeed(step: Step) {
    this.failures.delete(step);
  }

  /** Keeps matching calls pending, before they take effect, until released. */
  hold(step: Step, when = anyTarget) {
    let release!: () => void;
    const released = new Promise<void>((resolve) => {
      release = resolve;
    });
    this.holds.set(step, { when, released });
    return () => {
      this.holds.delete(step);
      release();
    };
  }

  targets(step: Step) {
    return this.calls.filter((call) => call.step === step).map((call) => call.target);
  }

  private async enter(step: Step, target: string) {
    this.calls.push({ step, target });
    const hold = this.holds.get(step);
    if (hold?.when(target)) await hold.released;
    if (this.failures.get(step)?.(target)) throw new Error(`fake ${step} failure`);
  }

  async validateRepos() {
    await this.enter("validateRepos", "");
  }

  async resolveManualWorktree(repo: WorkspaceRepo, rawName: string) {
    await this.enter("resolveManualWorktree", repo.localPath);
    return { branchName: `user/${rawName}`, path: `${repo.worktreeBasePath}/user/${rawName}` };
  }

  async validateWorktrees(
    members: TaskMember[],
    _basePaths: string[],
    ownedPaths: string[]
  ): Promise<WorktreeValidation> {
    if (members.some((member) => ownedPaths.includes(member.path))) return "owned";
    return this.enter("validateWorktrees", "").then(
      () => "valid",
      () => "invalid"
    );
  }

  async addWorktree(member: TaskMember) {
    await this.enter("addWorktree", member.path);
    this.worktrees.set(member.path, member);
    return { warning: null };
  }

  async readScripts(worktreePath: string) {
    await this.enter("readScripts", worktreePath);
    const member = this.worktrees.get(worktreePath);
    if (!member) return { ...NO_SCRIPTS, present: false };
    const scripts = this.scripts.get(member.localPath) ?? NO_SCRIPTS;
    if (typeof scripts === "string") throw scripts;
    return scripts;
  }

  async copyLocalConfigs(member: TaskMember, paths: string[], fallbackPaths: string[]) {
    await this.enter("copyLocalConfigs", member.path);
    this.copiedConfigs.set(member.path, [
      ...paths,
      ...(this.includes.get(member.localPath) ?? fallbackPaths),
    ]);
  }

  async prepareDetectedSetup(worktreePath: string): Promise<DetectedSetupPlan> {
    await this.enter("prepareDetectedSetup", worktreePath);
    return {
      worktreePath,
      steps: this.detectedSteps(worktreePath).map((step) => ({
        stage: step.stage,
        status: "pending",
      })),
    };
  }

  async runDetectedSetup(plan: DetectedSetupPlan, onStep: (step: DetectedStep) => void) {
    await this.enter("runDetectedSetup", plan.worktreePath);
    const steps = this.detectedSteps(plan.worktreePath);
    for (const step of steps) {
      onStep({ stage: step.stage, status: "running" });
      onStep(step);
    }
    return steps;
  }

  async runScript(run: TaskScriptRun) {
    await this.enter("runScript", run.cwd);
    this.scriptRuns.push(run);
    return this.exitCode(run);
  }

  async removeWorktree(member: TaskMember) {
    await this.enter("removeWorktree", member.path);
    this.worktrees.delete(member.path);
  }

  async removeTaskWorkspace(task: Task) {
    await this.enter("removeTaskWorkspace", task.id);
    this.removedTaskWorkspaces.push(task);
  }

  async cleanupRemovedTasks(tasks: Task[]) {
    await this.enter("cleanupRemovedTasks", "");
    this.cleanedTasks.push(...tasks);
    return this.failingCleanups;
  }
}
