import { invoke } from "@tauri-apps/api/core";
import { homeDir } from "@tauri-apps/api/path";
import { v4 as uuid } from "uuid";
import { Workspace, WorkspaceRepo, GitStatus } from "../types";
import { normalizeRepoScripts } from "../utils";
import { isCatalogAgent } from "./agents";
import { MarkedIssueInput } from "./linear";
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

export interface CliBridgeRequest {
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

function publicWorkspace(workspace: Workspace) {
  return {
    id: workspace.id,
    name: workspace.name,
    repos: workspace.repos.map(({ id, name, localPath, worktreeBasePath, scripts }) => ({
      id,
      name,
      localPath,
      worktreeBasePath,
      ...(scripts ? { scripts } : {}),
    })),
    linearOrgUrlKey: workspace.linearOrgUrlKey ?? null,
  };
}

function repoScriptsInput(value: unknown) {
  if (value === null) return undefined;
  const input = object(value, ["setup", "teardown"]);
  if (Object.values(input).some((script) => script !== null && typeof script !== "string"))
    throw new OperationError("invalid_params", "Repository scripts must be strings or null.");
  return normalizeRepoScripts(input);
}

async function workspaceInput(value: unknown, existing?: Workspace): Promise<Workspace> {
  const input = object(value, ["name", "repos", "linearOrgUrlKey"]);
  let repos = existing?.repos;
  if (input.repos !== undefined) {
    if (!Array.isArray(input.repos))
      throw new OperationError("invalid_params", "repos must be an array.");
    const home = (await homeDir()).replace(/\/$/, "");
    repos = input.repos.map((value): WorkspaceRepo => {
      const repo = object(value, ["id", "name", "localPath", "worktreeBasePath", "scripts"]);
      const localPath = text(repo.localPath).replace(/\/+$/, "");
      const name = repo.name === undefined ? localPath.split("/").pop()! : text(repo.name);
      const previous = existing?.repos.find((item) => item.localPath === localPath);
      const id = repo.id === undefined ? (previous?.id ?? uuid()) : text(repo.id);
      if (repo.id !== undefined && !existing?.repos.some((item) => item.id === id))
        throw new OperationError(
          "invalid_params",
          "Repository IDs may only reference existing members."
        );
      const scripts =
        repo.scripts === undefined ? previous?.scripts : repoScriptsInput(repo.scripts);
      return {
        id,
        name,
        localPath,
        ...(scripts ? { scripts } : {}),
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

function baseRefs(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new OperationError("invalid_params", "baseRefs must map repository ids to refs.");
  return Object.fromEntries(Object.entries(value).map(([repoId, ref]) => [repoId, text(ref)]));
}

function taskInput(value: unknown): CreateTaskInput {
  const input = object(value, [
    "workspaceId",
    "branchName",
    "repoIds",
    "linearIssue",
    "automationId",
    "baseRefs",
  ]);
  const result: CreateTaskInput = {
    workspaceId: text(input.workspaceId),
    branchName: text(input.branchName),
  };
  if (input.automationId !== undefined) result.automationId = text(input.automationId);
  if (input.baseRefs !== undefined) result.baseRefs = baseRefs(input.baseRefs);
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

async function agentEvent(params: Record<string, unknown>): Promise<AgentEvent> {
  const state = text(params.state);
  const agent = text(params.agent);
  if (!AGENT_ACTIVITY_STATES.includes(state as AgentActivityState))
    throw new OperationError("invalid_params", "state must be working, waiting or done.");
  if (!(await isCatalogAgent(agent)))
    throw new OperationError("invalid_params", `agent ${agent} is not in the agent catalog.`);
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
      throw new OperationError(
        "invalid_params",
        "surface must be chat, terminal, editor or opencode."
      );
    event.taskId = text(params.taskId);
    event.surface = surface as AgentSurfaceKind;
  }
  return event;
}

/** Operations only the automation engine in this process may run, never a socket client. */
const ENGINE_METHODS = ["linear.issue-upsert", "linear.issue-state", "task.log-append"];

function labelList(value: unknown): string[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new OperationError("invalid_params", "labels must be a list.");
  return value.map(text);
}

function markedIssueInput(params: Record<string, unknown>): MarkedIssueInput {
  return {
    marker: text(params.marker),
    team: text(params.team),
    title: text(params.title),
    project: params.project == null ? null : text(params.project),
    assignee: params.assignee === undefined ? "me" : text(params.assignee),
    labels: labelList(params.labels),
  };
}

const DEFAULT_RUNS_PAGE = 20;

function pageLimit(value: unknown): number {
  if (value === undefined) return DEFAULT_RUNS_PAGE;
  if (typeof value !== "number" || !Number.isInteger(value) || value < 1)
    throw new OperationError("invalid_params", "limit must be a positive integer.");
  return value;
}

/** Null goes back to the definition's value. */
function limitValue(value: unknown): number | null {
  if (value === null) return null;
  if (typeof value !== "number" || !Number.isInteger(value) || value < 1)
    throw new OperationError("invalid_params", "value must be a positive integer or null.");
  return value;
}

function payloadObject(value: unknown): Record<string, unknown> | null {
  if (value === undefined) return null;
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new OperationError("invalid_params", "payload must be a JSON object.");
  return value as Record<string, unknown>;
}

async function invokeAutomations(command: string, args?: Record<string, unknown>) {
  try {
    return await invoke(command, args);
  } catch (error) {
    throw new OperationError("operation_failed", String(error));
  }
}

function deleteOptions(params: Record<string, unknown>): DeleteOptions | undefined {
  if (params.deleteWorktrees === undefined) {
    if (params.force || params.skipTeardown)
      throw new OperationError(
        "invalid_params",
        "--force and --skip-teardown require --delete-worktrees."
      );
    return undefined;
  }
  if (
    typeof params.deleteWorktrees !== "boolean" ||
    (params.force !== undefined && typeof params.force !== "boolean") ||
    (params.skipTeardown !== undefined && typeof params.skipTeardown !== "boolean") ||
    ((params.force || params.skipTeardown) && !params.deleteWorktrees)
  ) {
    throw new OperationError(
      "invalid_params",
      "Choose a deletion mode; --force and --skip-teardown require --delete-worktrees."
    );
  }
  return {
    deleteWorktrees: params.deleteWorktrees,
    force: params.force === true,
    skipTeardown: params.skipTeardown === true,
  };
}

export async function dispatchCliBridge(
  operations: Operations,
  request: CliBridgeRequest,
  progress?: (message: string) => void,
  onAgentEvent?: (event: AgentEvent) => { taskId: string | null },
  engine = false
) {
  try {
    if (request.version !== 1)
      throw new OperationError("version_mismatch", "This app supports protocol version 1.");
    const allowed: Record<string, string[]> = {
      "workspace.list": [],
      "workspace.get": ["id"],
      "workspace.create": ["input"],
      "workspace.update": ["id", "input"],
      "workspace.delete": ["id", "deleteWorktrees", "force", "skipTeardown"],
      "task.list": ["workspaceId"],
      "task.get": ["id", "git"],
      "task.create": ["input"],
      "task.link-issue": ["id", "issue"],
      "task.delete": ["id", "deleteWorktrees", "force", "skipTeardown"],
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
      "automation.list": [],
      "automation.get": ["name"],
      "automation.runs": ["name", "before", "limit"],
      "automation.status": ["name", "runId"],
      "automation.run": ["name", "payload"],
      "automation.fire": ["name", "key", "payload"],
      "automation.pause": ["name", "reason"],
      "automation.resume": ["name", "reason"],
      "automation.items": ["name"],
      "automation.next": ["name"],
      "automation.continuous": ["name"],
      "automation.limit": ["name", "limit", "value"],
      "linear.issue-upsert": [
        "workspaceId",
        "marker",
        "team",
        "title",
        "project",
        "assignee",
        "labels",
      ],
      "linear.issue-state": ["workspaceId", "marker", "state"],
      "task.log-append": ["id", "entry", "key"],
    };
    if (
      !Object.hasOwn(allowed, request.method) ||
      (ENGINE_METHODS.includes(request.method) && !engine)
    )
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
        const event = await agentEvent(params);
        if (!onAgentEvent)
          throw new OperationError("app_not_ready", "Agent events are not handled yet.");
        result = onAgentEvent(event);
        break;
      }
      case "automation.list":
        result = await invokeAutomations("automations_list");
        break;
      case "automation.get":
        result = await invokeAutomations("automations_get", { name: text(params.name) });
        break;
      case "automation.runs":
        result = await invokeAutomations("automation_runs_list", {
          name: text(params.name),
          before: params.before === undefined ? null : text(params.before),
          limit: pageLimit(params.limit),
        });
        break;
      case "automation.status":
        result = await invokeAutomations("automation_run_get", {
          name: text(params.name),
          runId: text(params.runId),
        });
        break;
      case "automation.run":
        result = await invokeAutomations("automation_run", {
          name: text(params.name),
          test: false,
          payload: payloadObject(params.payload),
        });
        break;
      case "automation.fire":
        result = await invokeAutomations("automation_fire", {
          name: text(params.name),
          key: text(params.key),
          payload: payloadObject(params.payload),
        });
        break;
      case "automation.pause":
      case "automation.resume":
        result = await invokeAutomations(
          request.method === "automation.pause" ? "automation_pause" : "automation_resume",
          {
            name: text(params.name),
            by: "cli",
            reason: params.reason === undefined ? null : text(params.reason),
          }
        );
        break;
      case "automation.items":
        result = await invokeAutomations("automation_items_list", { name: text(params.name) });
        break;
      case "automation.next":
      case "automation.continuous":
        result = await invokeAutomations(
          request.method === "automation.next" ? "automation_next" : "automation_continuous",
          { name: text(params.name), by: "cli" }
        );
        break;
      case "automation.limit":
        result = await invokeAutomations("automation_set_limit", {
          name: text(params.name),
          by: "cli",
          limit: text(params.limit),
          value: limitValue(params.value),
        });
        break;
      case "linear.issue-upsert":
        result = await operations.upsertMarkedIssue(
          text(params.workspaceId),
          markedIssueInput(params)
        );
        break;
      case "linear.issue-state":
        result = await operations.moveMarkedIssue(
          text(params.workspaceId),
          text(params.marker),
          text(params.state)
        );
        break;
      case "task.log-append":
        result = await operations.appendTaskLog(
          text(params.id),
          text(params.entry),
          text(params.key)
        );
        break;
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
