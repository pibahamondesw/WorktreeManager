import { describe, expect, it, vi } from "vitest";
import { runDelete } from "./deletion";
import { OperationError } from "./operations";

const scripts = [{ repoName: "api", source: "worktreemanager" as const, script: "make stop" }];

function failingRemove(error: unknown) {
  return vi.fn().mockRejectedValue(error);
}

describe("runDelete", () => {
  it("returns warnings from a successful deletion", async () => {
    const warnings = [{ stage: "archive_note", message: "Could not archive the note" }];
    const remove = vi.fn().mockResolvedValue({ data: { id: "task" }, warnings });

    const outcome = await runDelete(remove, { deleteWorktrees: true });

    expect(outcome).toEqual({ kind: "deleted", warnings });
    expect(remove).toHaveBeenCalledWith({ deleteWorktrees: true, force: true });
  });

  it("passes the teardown choice through", async () => {
    const remove = vi.fn().mockResolvedValue({ data: { id: "task" }, warnings: [] });

    await runDelete(remove, { deleteWorktrees: true, teardown: { approveTeardown: true } });

    expect(remove).toHaveBeenCalledWith({
      deleteWorktrees: true,
      force: true,
      approveTeardown: true,
    });
  });

  it("asks for approval when teardown scripts are untrusted", async () => {
    const remove = failingRemove(
      new OperationError("teardown_needs_approval", "Review the scripts", { scripts })
    );

    expect(await runDelete(remove, { deleteWorktrees: true })).toEqual({
      kind: "teardown",
      teardown: "approval",
      message: "Before deleting, these repositories want to run a teardown script:",
      scripts,
    });
  });

  it("reports a teardown failure without scripts", async () => {
    const remove = failingRemove(
      new OperationError("teardown_failed", "Teardown failed for api.", { repoNames: ["api"] })
    );

    expect(await runDelete(remove, { deleteWorktrees: true })).toEqual({
      kind: "teardown",
      teardown: "failed",
      message: "Teardown failed for api.",
      scripts: [],
    });
  });

  it("offers removing from the app when disk deletion fails", async () => {
    const remove = failingRemove(new OperationError("delete_failed", "Some worktrees failed"));

    expect(await runDelete(remove, { deleteWorktrees: true })).toEqual({
      kind: "failed",
      message: "Some worktrees failed",
      canRemoveFromApp: true,
    });
  });

  it("does not offer removing from the app when that removal itself fails", async () => {
    const remove = failingRemove("store unavailable");

    expect(await runDelete(remove, { deleteWorktrees: false })).toEqual({
      kind: "failed",
      message: "store unavailable",
      canRemoveFromApp: false,
    });
    expect(remove).toHaveBeenCalledWith({ deleteWorktrees: false, force: false });
  });
});
