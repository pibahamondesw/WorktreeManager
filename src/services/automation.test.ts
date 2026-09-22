import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_STATE, AppState } from "../types";
import { Operations } from "./operations";
import { dispatchAutomation } from "./automation";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn().mockResolvedValue("/home/test/") }));
vi.mock("./store", () => ({ persist: vi.fn().mockResolvedValue(undefined) }));

let operations: Operations;
beforeEach(() => {
  let state: AppState = {
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
  dispatchAutomation(operations, { id: "request1", version, method, params });

describe("automation contract", () => {
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
    expect(await call("workspace.create", { input: null })).toMatchObject({
      ok: false,
      error: { code: "invalid_params" },
    });
  });
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
