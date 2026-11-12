import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_STATE, AppState } from "../types";
import { Operations } from "./operations";
import { dispatchCliBridge } from "./cliBridge";
import { invokeWithAgentCatalog } from "./agents.fixture";
import { LinearService } from "./linear";

const { invokeCommand } = vi.hoisted(() => ({
  invokeCommand: vi.fn<(command: string, args?: unknown) => Promise<unknown>>(() =>
    Promise.resolve(undefined)
  ),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: unknown) => invokeWithAgentCatalog(invokeCommand, command, args),
}));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn().mockResolvedValue("/home/test/") }));
vi.mock("./store", () => ({ persist: vi.fn().mockResolvedValue(undefined) }));

let operations: Operations;
let state: AppState;
beforeEach(() => {
  state = {
    ...DEFAULT_STATE,
    setup: { linearApiKey: "global-secret", isComplete: true },
    workspaces: [
      {
        id: "w1",
        name: "API",
        linearApiKey: "workspace-secret",
        repos: [{ id: "r1", name: "API", localPath: "/repos/api", worktreeBasePath: "/wt/api" }],
      },
    ],
  };
  operations = new Operations(
    () => state,
    (next) => {
      state = next;
    },
    () => "cursor"
  );
});
const call = (method: string, params: Record<string, unknown> = {}, version = 1) =>
  dispatchCliBridge(operations, { id: "request1", version, method, params });

describe("cli bridge contract", () => {
  it("redacts credentials and rejects credential fields and unknown methods", async () => {
    const listed = await call("workspace.list");
    expect(JSON.stringify(listed)).not.toContain("secret");
    expect(
      await call("workspace.update", { id: "w1", input: { linearApiKey: "new-secret" } })
    ).toMatchObject({ ok: false, error: { code: "invalid_params" } });
    expect(await call("task.update", { id: "t1" })).toMatchObject({
      ok: false,
      error: { code: "method_not_found" },
    });
    expect(await call("toString")).toMatchObject({ ok: false });
  });

  it("creates and edits workspace configuration while preserving IDs and credentials", async () => {
    const response = await call("workspace.create", {
      input: { repos: [{ localPath: "/repos/web" }] },
    });
    expect(response).toMatchObject({
      ok: true,
      data: {
        name: "web",
        repos: [{ worktreeBasePath: "/home/test/Documents/.worktreemanager/worktrees/web" }],
      },
    });
    const edited = await call("workspace.update", {
      id: "w1",
      input: { name: "Renamed", repos: [{ localPath: "/repos/api" }] },
    });
    expect(edited).toMatchObject({ ok: true, data: { name: "Renamed", repos: [{ id: "r1" }] } });
    expect(operations.workspace("w1").linearApiKey).toBe("workspace-secret");
    expect(JSON.stringify(edited)).not.toContain("secret");
  });

  it("sets, keeps and clears repository script overrides", async () => {
    const update = (repo: Record<string, unknown>) =>
      call("workspace.update", {
        id: "w1",
        input: { repos: [{ localPath: "/repos/api", ...repo }] },
      });
    expect(await update({ scripts: { setup: " pnpm i ", teardown: "" } })).toMatchObject({
      ok: true,
      data: { repos: [{ scripts: { setup: "pnpm i" } }] },
    });
    await update({ name: "Renamed" });
    expect(operations.workspace("w1").repos[0].scripts).toEqual({ setup: "pnpm i" });
    expect(await update({ scripts: { setup: 1 } })).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
    await update({ scripts: null });
    expect(operations.workspace("w1").repos[0]).not.toHaveProperty("scripts");
  });

  it("rejects mismatched protocol, IDs, flags, and malformed inputs", async () => {
    expect(await call("workspace.list", {}, 2)).toMatchObject({
      ok: false,
      error: { code: "version_mismatch" },
    });
    expect(await call("workspace.get", { id: "missing" })).toMatchObject({
      ok: false,
      error: { code: "not_found" },
    });
    expect(await call("task.list", { workspaceId: "missing" })).toMatchObject({
      ok: false,
      error: { code: "not_found" },
    });
    expect(await call("workspace.list", { git: true })).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
    expect(
      await call("task.create", {
        input: { workspaceId: "w1", branchName: "foo", repoIds: ["r1", "r1"] },
      })
    ).toMatchObject({ ok: false, error: { code: "invalid_params" } });
    expect(
      await call("task.delete", { id: "t1", deleteWorktrees: false, force: true })
    ).toMatchObject({ ok: false, error: { code: "invalid_params" } });
    expect(
      await call("task.delete", { id: "t1", deleteWorktrees: false, skipTeardown: true })
    ).toMatchObject({ ok: false, error: { code: "invalid_params" } });
    expect(await call("workspace.create", { input: null })).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
  });
});

it("passes the creating automation through task creation", async () => {
  const create = vi
    .spyOn(operations, "createTask")
    .mockResolvedValue({ data: { id: "t1" }, warnings: [] } as never);
  const input = { workspaceId: "w1", branchName: "wtm/demo/1a2b3c4d", automationId: "demo" };
  expect(await call("task.create", { input })).toMatchObject({ ok: true, data: { id: "t1" } });
  expect(create.mock.calls[0][0]).toEqual(input);
  expect(await call("task.create", { input: { ...input, automationId: " " } })).toMatchObject({
    ok: false,
    error: { code: "invalid_params" },
  });
  const based = { ...input, baseRefs: { r1: "origin/develop" } };
  expect(await call("task.create", { input: based })).toMatchObject({ ok: true });
  expect(create.mock.calls[1][0]).toEqual(based);
  for (const refs of [["origin/develop"], { r1: " " }, null])
    expect(await call("task.create", { input: { ...input, baseRefs: refs } })).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
  create.mockRestore();
});

it("routes issue linking through the shared operation and validates its parameters", async () => {
  const link = vi
    .spyOn(operations, "linkTaskIssue")
    .mockResolvedValue({ id: "t1", linearIssueIdentifier: "WOR-123" } as never);
  expect(await call("task.link-issue", { id: "t1", issue: " WOR-123 " })).toMatchObject({
    ok: true,
    data: { id: "t1", linearIssueIdentifier: "WOR-123" },
  });
  expect(link).toHaveBeenCalledWith("t1", "WOR-123");
  for (const params of [
    { id: "t1" },
    { id: "t1", issue: " " },
    { id: "t1", issue: 123 },
    { id: "t1", issue: "WOR-123", force: true },
  ]) {
    expect(await call("task.link-issue", params)).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
  }
  expect(link).toHaveBeenCalledOnce();
});

it("forwards validated agent events to the app and reports the matched task", async () => {
  const onAgentEvent = vi.fn(() => ({ taskId: "t1" }));
  const event = (params: Record<string, unknown>) =>
    dispatchCliBridge(
      operations,
      { id: "request1", version: 1, method: "agent.event", params },
      undefined,
      onAgentEvent
    );

  expect(
    await event({ state: "waiting", agent: "claude", cwd: "/wt/api/t1", at: 5 })
  ).toMatchObject({ ok: true, data: { taskId: "t1" } });
  expect(onAgentEvent).toHaveBeenLastCalledWith({
    state: "waiting",
    agent: "claude",
    cwd: "/wt/api/t1",
    at: 5,
    surface: "external",
  });
  await event({
    state: "done",
    agent: "codex",
    cwd: "/wt",
    at: 6,
    taskId: "t1",
    surface: "editor",
  });
  expect(onAgentEvent).toHaveBeenLastCalledWith(
    expect.objectContaining({ taskId: "t1", surface: "editor" })
  );
  await event({ state: "waiting", agent: "codex", cwd: "/wt", at: 7, asyncQuestion: true });
  expect(onAgentEvent).toHaveBeenLastCalledWith(expect.objectContaining({ asyncQuestion: true }));
  for (const params of [
    { state: "idle", agent: "claude", cwd: "/wt", at: 1 },
    { state: "done", agent: "cursor", cwd: "/wt", at: 1 },
    { state: "done", agent: "codex", at: 1 },
    { state: "done", agent: "codex", cwd: "/wt" },
    { state: "done", agent: "codex", cwd: "/wt", at: 1, extra: true },
    { state: "done", agent: "codex", cwd: "/wt", at: 1, taskId: "t1" },
    { state: "done", agent: "codex", cwd: "/wt", at: 1, taskId: "t1", surface: "external" },
    { state: "working", agent: "codex", cwd: "/wt", at: 1, prompt: false },
  ])
    expect(await event(params)).toMatchObject({ ok: false, error: { code: "invalid_params" } });
  expect(onAgentEvent).toHaveBeenCalledTimes(3);
  expect(
    await call("agent.event", { state: "done", agent: "codex", cwd: "/wt", at: 1 })
  ).toMatchObject({
    ok: false,
    error: { code: "app_not_ready" },
  });
});

it("reads automations and their runs through the Rust commands", async () => {
  invokeCommand.mockImplementation((command) =>
    command === "automation_run_get"
      ? Promise.reject("No run r9 for automation babysitter")
      : Promise.resolve({ command })
  );
  expect(await call("automation.list")).toMatchObject({
    ok: true,
    data: { command: "automations_list" },
  });
  expect(await call("automation.get", { name: " babysitter " })).toMatchObject({ ok: true });
  expect(invokeCommand).toHaveBeenLastCalledWith("automations_get", { name: "babysitter" });
  await call("automation.runs", { name: "babysitter" });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_runs_list", {
    name: "babysitter",
    before: null,
    limit: 20,
  });
  await call("automation.runs", { name: "babysitter", before: "r5", limit: 5 });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_runs_list", {
    name: "babysitter",
    before: "r5",
    limit: 5,
  });
  expect(await call("automation.status", { name: "babysitter", runId: "r9" })).toEqual({
    ok: false,
    error: { code: "operation_failed", message: "No run r9 for automation babysitter" },
  });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_run_get", {
    name: "babysitter",
    runId: "r9",
  });
  const calls = invokeCommand.mock.calls.length;
  for (const [method, params] of [
    ["automation.list", { name: "babysitter" }],
    ["automation.get", {}],
    ["automation.runs", { name: "babysitter", limit: 0 }],
    ["automation.runs", { name: "babysitter", limit: 2.5 }],
    ["automation.runs", { name: "babysitter", limit: "5" }],
    ["automation.runs", { name: "babysitter", before: " " }],
    ["automation.status", { name: "babysitter" }],
  ] as const)
    expect(await call(method, params)).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
  expect(invokeCommand).toHaveBeenCalledTimes(calls);
});

it("signals a waiting step for the CLI", async () => {
  invokeCommand.mockImplementation((command) => Promise.resolve({ command }));
  const payload = { decision: "approve" };
  expect(await call("automation.signal", { runId: "r1", step: "ship", payload })).toMatchObject({
    ok: true,
  });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_signal", {
    signal: { name: null, runId: "r1", step: "ship", item: null, payload, by: "cli" },
  });
  await call("automation.signal", { runId: "r1", step: "go", item: "a" });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_signal", {
    signal: { name: null, runId: "r1", step: "go", item: "a", payload: {}, by: "cli" },
  });
  expect(await call("automation.signal", { runId: "r1", step: "go", payload: [1] })).toMatchObject({
    ok: false,
    error: { code: "invalid_params" },
  });
});

it("takes over, hands back and retries items for the CLI", async () => {
  invokeCommand.mockImplementation((command) => Promise.resolve({ command }));
  const target = (item: string | null) => ({ name: null, runId: "r1", item, by: "cli" });

  expect(await call("automation.take-over", { runId: "r1" })).toMatchObject({ ok: true });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_take_over", { target: target(null) });
  await call("automation.hand-back", { runId: "r1", item: "a", note: "Use the API" });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_hand_back", {
    target: target("a"),
    note: "Use the API",
  });
  await call("automation.hand-back", { runId: "r1" });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_hand_back", {
    target: target(null),
    note: null,
  });
  await call("automation.retry", { runId: "r1" });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_retry", {
    target: target(null),
    definition: "current",
    from: null,
  });
  await call("automation.retry", {
    runId: "r1",
    item: "a",
    definition: "original",
    from: "implement",
  });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_retry", {
    target: target("a"),
    definition: "original",
    from: "implement",
  });
});

it("starts, pauses and resumes automations for the CLI", async () => {
  invokeCommand.mockImplementation((command) => Promise.resolve({ command }));
  expect(await call("automation.run", { name: " babysitter " })).toMatchObject({
    ok: true,
    data: { command: "automation_run" },
  });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_run", {
    name: "babysitter",
    test: false,
    payload: null,
  });
  await call("automation.run", { name: "babysitter", payload: { pr: 7 } });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_run", {
    name: "babysitter",
    test: false,
    payload: { pr: 7 },
  });
  await call("automation.run", { name: "babysitter", test: true });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_run", {
    name: "babysitter",
    test: true,
    payload: null,
  });
  await call("automation.fire", { name: "babysitter", key: " pr:7 ", payload: { pr: 7 } });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_fire", {
    name: "babysitter",
    key: "pr:7",
    payload: { pr: 7 },
  });
  await call("automation.pause", { name: "babysitter", reason: " vacation " });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_pause", {
    name: "babysitter",
    by: "cli",
    reason: "vacation",
  });
  await call("automation.resume", { name: "babysitter" });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_resume", {
    name: "babysitter",
    by: "cli",
    reason: null,
  });
  const calls = invokeCommand.mock.calls.length;
  for (const [method, params] of [
    ["automation.run", { name: "babysitter", payload: [1] }],
    ["automation.run", { name: "babysitter", payload: null }],
    ["automation.fire", { name: "babysitter" }],
    ["automation.fire", { name: "babysitter", key: "pr:7", payload: "{}" }],
    ["automation.pause", { name: "babysitter", reason: " " }],
    ["automation.resume", { name: "babysitter", test: true }],
  ] as const)
    expect(await call(method, params)).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
  expect(invokeCommand).toHaveBeenCalledTimes(calls);
});

it("lists campaign items and applies operator verbs for the CLI", async () => {
  invokeCommand.mockImplementation((command) => Promise.resolve({ command }));
  expect(await call("automation.items", { name: " lint-sweep " })).toMatchObject({
    ok: true,
    data: { command: "automation_items_list" },
  });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_items_list", { name: "lint-sweep" });
  await call("automation.next", { name: "lint-sweep" });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_next", {
    name: "lint-sweep",
    by: "cli",
  });
  await call("automation.continuous", { name: "lint-sweep" });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_continuous", {
    name: "lint-sweep",
    by: "cli",
  });
  await call("automation.limit", { name: "lint-sweep", limit: "max_active", value: 3 });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_set_limit", {
    name: "lint-sweep",
    by: "cli",
    limit: "max_active",
    value: 3,
  });
  await call("automation.limit", { name: "lint-sweep", limit: "batch_size", value: null });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_set_limit", {
    name: "lint-sweep",
    by: "cli",
    limit: "batch_size",
    value: null,
  });
  const calls = invokeCommand.mock.calls.length;
  for (const params of [
    { name: "lint-sweep", limit: "max_active", value: 0 },
    { name: "lint-sweep", limit: "max_active", value: "3" },
    { name: "lint-sweep", limit: "max_active" },
    { name: "lint-sweep", value: 2 },
  ])
    expect(await call("automation.limit", params)).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
  expect(invokeCommand).toHaveBeenCalledTimes(calls);
});

it("records, lists and revokes decisions for the CLI", async () => {
  invokeCommand.mockResolvedValueOnce({ steered: 1 });
  expect(
    await call("automation.decide", { name: "lint-sweep", text: " Never use define_method " })
  ).toMatchObject({ ok: true, data: { steered: 1 } });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_decide", {
    name: "lint-sweep",
    text: "Never use define_method",
    by: "cli",
  });
  const decision = { id: "1a2b3c4d", text: "Rebase", by: "user", at: "2026-11-01T10:00:00Z" };
  invokeCommand.mockResolvedValueOnce({ enabled: true, decisions: [decision] });
  expect(await call("automation.decisions", { name: "lint-sweep" })).toMatchObject({
    ok: true,
    data: [decision],
  });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_state", { name: "lint-sweep" });
  invokeCommand.mockResolvedValueOnce({ enabled: true });
  expect(await call("automation.decisions", { name: "lint-sweep" })).toMatchObject({
    ok: true,
    data: [],
  });
  await call("automation.revoke", { name: "lint-sweep", id: "1a2b3c4d" });
  expect(invokeCommand).toHaveBeenLastCalledWith("automation_revoke_decision", {
    name: "lint-sweep",
    id: "1a2b3c4d",
  });
  expect(await call("automation.decide", { name: "lint-sweep", text: " " })).toMatchObject({
    ok: false,
    error: { code: "invalid_params" },
  });
});

describe("engine-only operations", () => {
  const engineCall = (method: string, params: Record<string, unknown>) =>
    dispatchCliBridge(
      operations,
      { id: "request1", version: 1, method, params },
      undefined,
      undefined,
      true
    );
  const upsert = {
    workspaceId: "w1",
    marker: "m:1",
    team: "ENG",
    title: " Lint ",
    project: null,
    labels: ["Lint"],
  };

  it("are unknown to socket clients", async () => {
    for (const method of ["linear.issue-upsert", "linear.issue-state", "task.log-append"])
      expect(await call(method, {})).toMatchObject({
        ok: false,
        error: { code: "method_not_found" },
      });
  });

  it("upsert and move marked Linear issues with the workspace credential", async () => {
    const upsertIssue = vi.spyOn(LinearService.prototype, "upsertMarkedIssue").mockResolvedValue({
      id: "i",
      identifier: "ENG-1",
      title: "Lint",
      url: "u",
      branchName: "me/dx-1",
      created: true,
      updated: false,
      warnings: [],
    });
    const moveIssue = vi
      .spyOn(LinearService.prototype, "moveMarkedIssue")
      .mockRejectedValueOnce(new Error("Linear state Shipped not found in the issue's team"))
      .mockRejectedValueOnce("offline");

    expect(await engineCall("linear.issue-upsert", upsert)).toMatchObject({
      ok: true,
      data: { identifier: "ENG-1", title: "Lint", created: true },
    });
    expect(upsertIssue).toHaveBeenCalledWith({
      marker: "m:1",
      team: "ENG",
      title: "Lint",
      project: null,
      assignee: "me",
      labels: ["Lint"],
      milestone: null,
    });
    await engineCall("linear.issue-upsert", {
      ...upsert,
      project: "P",
      assignee: "ana",
      labels: undefined,
    });
    expect(upsertIssue).toHaveBeenLastCalledWith(
      expect.objectContaining({ project: "P", assignee: "ana", labels: [] })
    );
    await engineCall("linear.issue-upsert", { ...upsert, labels: ["Lint", ""], milestone: "" });
    expect(upsertIssue).toHaveBeenLastCalledWith(
      expect.objectContaining({ labels: ["Lint", ""], milestone: "" })
    );
    expect(await engineCall("linear.issue-upsert", { ...upsert, milestone: 1 })).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
    expect(await engineCall("linear.issue-upsert", { ...upsert, labels: "Lint" })).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });

    const state = { workspaceId: "w1", marker: "m:1", state: "Shipped" };
    expect(await engineCall("linear.issue-state", state)).toEqual({
      ok: false,
      error: {
        code: "linear_failed",
        message: "Linear state Shipped not found in the issue's team",
      },
    });
    expect(await engineCall("linear.issue-state", state)).toMatchObject({
      ok: false,
      error: { code: "linear_failed", message: "The Linear request failed." },
    });
    expect(JSON.stringify(upsertIssue.mock.calls)).not.toContain("secret");
    upsertIssue.mockRestore();
    moveIssue.mockRestore();
  });

  it("need Linear configured in the workspace", async () => {
    state.workspaces[0].linearApiKey = undefined;
    expect(await engineCall("linear.issue-upsert", upsert)).toMatchObject({
      ok: false,
      error: { code: "linear_not_configured" },
    });
  });

  it("append to the task log of an existing task", async () => {
    state.tasks = [
      {
        id: "t1",
        workspaceId: "w1",
        branchName: "wtm/demo/1",
        members: [],
        createdAt: "2026-11-03T00:00:00.000Z",
      },
    ];
    const entry = { id: "t1", entry: "Integrated", key: "wtm:k" };
    expect(await engineCall("task.log-append", entry)).toEqual({
      ok: true,
      data: { status: "vault_disabled", path: null },
      warnings: [],
    });
    expect(await engineCall("task.log-append", { ...entry, id: "t2" })).toMatchObject({
      ok: false,
      error: { code: "not_found" },
    });
  });
});
