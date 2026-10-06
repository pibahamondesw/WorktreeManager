import { invoke } from "@tauri-apps/api/core";
import { Task, TaskMember, Workspace, WorkspaceRepo } from "../types";
import { loadScriptApprovals, persist } from "./store";

export type ScriptPhase = "setup" | "teardown";
type RepoScriptSource = "worktreemanager" | "conductor" | "superset" | "cursor";
export type ScriptSource = "local" | RepoScriptSource;

export const SCRIPT_SOURCE_LABELS: Record<ScriptSource, string> = {
  local: "app override",
  worktreemanager: ".worktreemanager.toml",
  conductor: "Conductor config",
  superset: "Superset config",
  cursor: ".cursor/worktrees.json",
};

/** What a checkout declares, as read by `resolve_repo_scripts`. */
export interface ResolvedScripts {
  present: boolean;
  source: RepoScriptSource | null;
  setup: string | null;
  setupHash: string | null;
  teardown: string | null;
  teardownHash: string | null;
}

export interface PhaseScript {
  source: ScriptSource;
  script: string;
  hash?: string;
}

export const scriptSession = (phase: ScriptPhase, repoId: string) => `${phase}:${repoId}`;

/** The app override wins for each phase it sets; otherwise the repository's declaration. */
export function phaseScript(
  repo: WorkspaceRepo | undefined,
  resolved: ResolvedScripts,
  phase: ScriptPhase
): PhaseScript | null {
  const override = repo?.scripts?.[phase]?.trim();
  if (override) return { source: "local", script: override };
  const script = resolved[phase];
  if (!script || !resolved.source) return null;
  return {
    source: resolved.source,
    script,
    hash: resolved[`${phase}Hash`] ?? undefined,
  };
}

export const resolveRepoScripts = (worktreePath: string) =>
  invoke<ResolvedScripts>("resolve_repo_scripts", { worktreePath });

let approvals: Promise<Record<string, string[]>> | undefined;

function loadApprovals() {
  approvals ??= loadScriptApprovals().catch((error) => {
    approvals = undefined;
    throw error;
  });
  return approvals;
}

/** Overrides are the user's own; repository scripts run once their exact content is approved. */
export async function isScriptTrusted(localPath: string, script: PhaseScript) {
  if (script.source === "local") return true;
  return !!script.hash && (await loadApprovals())[localPath]?.includes(script.hash) === true;
}

export async function approveScripts(localPath: string, hashes: string[]) {
  const current = await loadApprovals();
  const next = {
    ...current,
    [localPath]: [...new Set([...(current[localPath] ?? []), ...hashes])],
  };
  await persist([["scriptApprovals", next]]);
  approvals = Promise.resolve(next);
}

export function resetScriptApprovalsCache() {
  approvals = undefined;
}

/**
 * Context for repository scripts, passed only as environment variables. Conductor, Cursor and
 * Superset names are aliased so scripts written for them run unchanged.
 */
export function scriptEnv(task: Task, member: TaskMember, workspace: Workspace | undefined) {
  const worktrees = Object.fromEntries(task.members.map((item) => [item.repoName, item.path]));
  const workspaceName = workspace?.name ?? "";
  return Object.entries({
    WTM_ROOT_PATH: member.localPath,
    WTM_WORKTREE_PATH: member.path,
    WTM_TASK_ID: task.id,
    WTM_BRANCH: member.branchName,
    WTM_REPO_NAME: member.repoName,
    WTM_WORKSPACE_NAME: workspaceName,
    WTM_ISSUE_ID: task.linearIssueIdentifier ?? "",
    WTM_TASK_WORKTREES: JSON.stringify(worktrees),
    CONDUCTOR_ROOT_PATH: member.localPath,
    CONDUCTOR_WORKSPACE_PATH: member.path,
    CONDUCTOR_WORKSPACE_NAME: member.branchName,
    ROOT_WORKTREE_PATH: member.localPath,
    SUPERSET_ROOT_PATH: member.localPath,
    SUPERSET_WORKSPACE_PATH: member.path,
    SUPERSET_WORKSPACE_NAME: member.branchName,
  });
}

/** A deletion stopped by repository teardown scripts, as the UI offers to resolve it. */
export interface TeardownBlock {
  kind: "approval" | "failed";
  message: string;
  scripts: { repoName: string; source: ScriptSource; script: string }[];
}

export function teardownBlockFrom(error: unknown): TeardownBlock | null {
  if (!(error instanceof Error) || !("code" in error)) return null;
  if (error.code === "teardown_needs_approval")
    return {
      kind: "approval",
      message: "Before deleting, these repositories want to run a teardown script:",
      scripts: (error as unknown as { details: { scripts: TeardownBlock["scripts"] } }).details
        .scripts,
    };
  if (error.code === "teardown_failed")
    return { kind: "failed", message: error.message, scripts: [] };
  return null;
}
