import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { Task } from "../types";
import { tauriWorktreeLifecycle } from "./worktreeLifecycle";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), Channel: class {} }));

const member = {
  repoId: "api",
  repoName: "api",
  localPath: "/repos/api",
  path: "/wt/api/feature",
  branchName: "feature",
};
const task: Task = {
  id: "t1",
  workspaceId: "w1",
  branchName: "feature",
  createdAt: "2026-10-27",
  members: [member],
};

beforeEach(() => {
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
});

describe("tauriWorktreeLifecycle", () => {
  it.each([
    [undefined, "valid"],
    ["owned", "owned"],
    ["invalid", "invalid"],
    [new Error("ipc"), "invalid"],
  ])("maps worktree validation %s to %s", async (error, expected) => {
    if (error) vi.mocked(invoke).mockRejectedValueOnce(error);
    await expect(tauriWorktreeLifecycle.validateWorktrees([member], ["/wt/api"], [])).resolves.toBe(
      expected
    );
  });

  it("reports every cleanup as failed when the command fails and still closes terminals", async () => {
    vi.mocked(invoke).mockRejectedValueOnce(new Error("ipc"));
    await expect(tauriWorktreeLifecycle.cleanupRemovedTasks([task])).resolves.toEqual([
      "cleanup_claude_json",
      "doppler_cleanup",
    ]);
    expect(invoke).toHaveBeenCalledWith("cleanup_removed_worktrees", { paths: [member.path] });
    expect(invoke).toHaveBeenCalledWith("terminal_close", { taskId: "t1" });
  });
});
