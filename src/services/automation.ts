import { invoke } from "@tauri-apps/api/core";
import { homeDir } from "@tauri-apps/api/path";
import { v4 as uuid } from "uuid";
import { Workspace, WorkspaceRepo, GitStatus } from "../types";
import {
  Operations,
  OperationError,
  operationError,
  DeleteOptions,
  CreateTaskInput,
} from "./operations";
import {
  AGENT_ACTIVITY_STATES,
  AgentActivityState,
  AgentEvent,
  EMBEDDED_SURFACES,
  AgentSurfaceKind,
} from "./agentActivity";

export interface AutomationRequest {
  id: string;
  version: number;
  method: string;
  params: Record<string, unknown>;
}

function object(value: unknown, keys: string[]): Record<string, unknown> {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).some((key) => !keys.includes(key))
  ) {
    throw new OperationError(
      "invalid_params",
      "Invalid object or unsupported fields. See wtm --help."
    );
  }
  return value as Record<string, unknown>;
}

function text(value: unknown): string {
  if (typeof value !== "string" || !value.trim())
    throw new OperationError("invalid_params", "A required string is missing or empty.");
  return value.trim();
}

export function publicWorkspace(workspace: Workspace) {
  return {
    id: workspace.id,
    name: workspace.name,
    repos: workspace.repos.map(({ id, name, localPath, worktreeBasePath }) => ({
      id,
      name,
      localPath,
      worktreeBasePath,
    })),
    linearOrgUrlKey: workspace.linearOrgUrlKey ?? null,
  };
}

async function workspaceInput(value: unknown, existing?: Workspace): Promise<Workspace> {
  const input = object(value, ["name", "repos", "linearOrgUrlKey"]);
  let repos = existing?.repos;
  if (input.repos !== undefined) {
    if (!Array.isArray(input.repos))
      throw new OperationError("invalid_params", "repos must be an array.");
    const home = (await homeDir()).replace(/\/$/, "");
    repos = input.repos.map((value): WorkspaceRepo => {
      const repo = object(value, ["id", "name", "localPath", "worktreeBasePath"]);
      const localPath = text(repo.localPath).replace(/\/+$/, "");
      const name = repo.name === undefined ? localPath.split("/").pop()! : text(repo.name);
      const previous = existing?.repos.find((item) => item.localPath === localPath);
      const id = repo.id === undefined ? (previous?.id ?? uuid()) : text(repo.id);
      if (repo.id !== undefined && !existing?.repos.some((item) => item.id === id))
        throw new OperationError(
          "invalid_params",
          "Repository IDs may only reference existing members."
        );
      return {
        id,
        name,
        localPath,
        worktreeBasePath:
          repo.worktreeBasePath === undefined
            ? (previous?.worktreeBasePath ??
              `${home}/Documents/.worktreemanager/worktrees/${name
                .toLowerCase()
                .replace(/[^a-z0-9]+/g, "-")
                .replace(/^-|-$/g, "")}`)
            : text(repo.worktreeBasePath),
      };
    });
  }
  if (!repos?.length)
    throw new OperationError("invalid_params", "Provide at least one repository.");
  return {
    ...existing,
    id: existing?.id ?? uuid(),
    name:
      input.name === undefined
        ? (existing?.name ?? (repos.length === 1 ? repos[0].name : ""))
        : text(input.name),
    repos,
    ...(input.linearOrgUrlKey !== undefined
      ? { linearOrgUrlKey: input.linearOrgUrlKey === null ? null : text(input.linearOrgUrlKey) }
      : {}),
  };
}

function taskInput(value: unknown): CreateTaskInput {
  const input = object(value, ["workspaceId", "branchName", "repoIds", "linearIssue"]);
  const result: CreateTaskInput = {
    workspaceId: text(input.workspaceId),
    branchName: text(input.branchName),
  };
  if (input.repoIds !== undefined) {
    if (!Array.isArray(input.repoIds))
      throw new OperationError("invalid_params", "repoIds must be an array.");
    result.repoIds = input.repoIds.map(text);
    if (new Set(result.repoIds).size !== result.repoIds.length)
      throw new OperationError("invalid_params", "repoIds must be unique.");
  }
  if (input.linearIssue !== undefined) {
    const issue = object(input.linearIssue, ["id", "identifier", "title"]);
    result.linearIssue = {
      id: text(issue.id),
      identifier: text(issue.identifier),
      title: text(issue.title),
    };
  }
  return result;
}

function agentEvent(params: Record<string, unknown>): AgentEvent {
  const state = text(params.state);
  const agent = text(params.agent);
  if (!AGENT_ACTIVITY_STATES.includes(state as AgentActivityState))
    throw new OperationError("invalid_params", "state must be working, waiting or done.");
  if (agent !== "claude" && agent !== "codex")
    throw new OperationError("invalid_params", "agent must be claude or codex.");
  if (typeof params.at !== "number" || !Number.isFinite(params.at))
    throw new OperationError("invalid_params", "at must be a timestamp.");
  const event: AgentEvent = {
    state: state as AgentActivityState,
    agent,
    cwd: text(params.cwd),
    at: params.at,
    surface: "external",
  };
  for (const flag of ["prompt", "asyncQuestion"] as const) {
    if (params[flag] === undefined) continue;
    if (params[flag] !== true)
      throw new OperationError("invalid_params", `${flag} must be true when present.`);
    event[flag] = true;
  }
  if (params.taskId !== undefined || params.surface !== undefined) {
    const surface = text(params.surface);
    if (!EMBEDDED_SURFACES.includes(surface as AgentSurfaceKind))
      throw new OperationError("invalid_params", "surface must be chat, terminal or editor.");
    event.taskId = text(params.taskId);
    event.surface = surface as AgentSurfaceKind;
  }
  return event;
}

function deleteOptions(params: Record<string, unknown>): DeleteOptions | undefined {
  if (params.deleteWorktrees === undefined) {
    if (params.force)
      throw new OperationError("invalid_params", "--force requires --delete-worktrees.");
    return undefined;
  }
  if (
    typeof params.deleteWorktrees !== "boolean" ||
    (params.force !== undefined && typeof params.force !== "boolean") ||
    (params.force && !params.deleteWorktrees)
  ) {
    throw new OperationError(
      "invalid_params",
      "Choose a deletion mode; --force requires --delete-worktrees."
    );
  }
  return { deleteWorktrees: params.deleteWorktrees, force: params.force === true };
}

export async function dispatchAutomation(
  operations: Operations,
  request: AutomationRequest,
  progress?: (message: string) => void,
  onAgentEvent?: (event: AgentEvent) => { taskId: string | null }
) {
  try {
    if (request.version !== 1)
      throw new OperationError("version_mismatch", "This app supports protocol version 1.");
    const allowed: Record<string, string[]> = {
      "workspace.list": [],
      "workspace.get": ["id"],
      "workspace.create": ["input"],
      "workspace.update": ["id", "input"],
      "workspace.delete": ["id", "deleteWorktrees", "force"],
      "task.list": ["workspaceId"],
      "task.get": ["id", "git"],
      "task.create": ["input"],
      "task.link-issue": ["id", "issue"],
      "task.delete": ["id", "deleteWorktrees", "force"],
      "agent.event": [
        "state",
        "agent",
        "cwd",
        "at",
        "taskId",
        "surface",
        "prompt",
        "asyncQuestion",
      ],
    };
    if (!Object.hasOwn(allowed, request.method))
      throw new OperationError("method_not_found", "Unknown operation. See wtm --help.");
    const params = object(request.params, allowed[request.method]);
    let result: unknown;
    switch (request.method) {
      case "workspace.list":
        result = operations.getState().workspaces.map(publicWorkspace);
        break;
      case "workspace.get":
        result = publicWorkspace(operations.workspace(text(params.id)));
        break;
      case "workspace.create":
        result = publicWorkspace(
          await operations.addWorkspace(await workspaceInput(params.input), false)
        );
        break;
      case "workspace.update": {
        const id = text(params.id);
        const input = await workspaceInput(params.input, operations.workspace(id));
        const patch = object(params.input, ["name", "repos", "linearOrgUrlKey"]);
        result = publicWorkspace(
          await operations.updateWorkspace(id, {
            ...(patch.name !== undefined ? { name: input.name } : {}),
            ...(patch.repos !== undefined ? { repos: input.repos } : {}),
            ...(patch.linearOrgUrlKey !== undefined
              ? { linearOrgUrlKey: input.linearOrgUrlKey }
              : {}),
          })
        );
        break;
      }
      case "workspace.delete":
        return {
          ok: true,
          ...(await operations.deleteWorkspace(text(params.id), deleteOptions(params))),
        };
      case "task.list": {
        const id = params.workspaceId === undefined ? undefined : text(params.workspaceId);
        if (id) operations.workspace(id);
        result = operations.getState().tasks.filter((task) => !id || task.workspaceId === id);
        break;
      }
      case "task.get": {
        if (params.git !== undefined && typeof params.git !== "boolean")
          throw new OperationError("invalid_params", "git must be a boolean.");
        const task = operations.task(text(params.id));
        if (!params.git) {
          result = task;
          break;
        }
        const git: Record<string, GitStatus | null> = {};
        for (const member of task.members) {
          git[member.repoId] = await invoke<GitStatus>("git_worktree_status", {
            repoPath: member.localPath,
            worktreePath: member.path,
          }).catch(() => null);
        }
        result = { ...task, git };
        break;
      }
      case "task.link-issue":
        result = await operations.linkTaskIssue(text(params.id), text(params.issue));
        break;
      case "task.create":
        return { ok: true, ...(await operations.createTask(taskInput(params.input), progress)) };
      case "task.delete": {
        const options = deleteOptions(params);
        if (!options)
          throw new OperationError(
            "invalid_params",
            "Choose --keep-worktrees or --delete-worktrees."
          );
        return { ok: true, ...(await operations.deleteTask(text(params.id), options)) };
      }
      case "agent.event": {
        const event = agentEvent(params);
        if (!onAgentEvent)
          throw new OperationError("app_not_ready", "Agent events are not handled yet.");
        result = onAgentEvent(event);
        break;
      }
    }
    return { ok: true, data: result, warnings: [] };
  } catch (error) {
    const failure = operationError(error);
    return {
      ok: false,
      error: {
        code: failure.code,
        message: failure.message,
        ...(failure.details ? { details: failure.details } : {}),
      },
    };
  }
}
