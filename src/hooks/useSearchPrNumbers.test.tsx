// @vitest-environment jsdom
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { LinearService } from "../services/linear";
import { taskLinearStore } from "../services/taskLinear";
import { resetGithubCache } from "../services/github";
import { Task, Workspace } from "../types";
import { useWorktreeData } from "./useWorktreeData";
import { useSearchPrNumbers } from "./useSearchPrNumbers";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const pr = {
  number: 12,
  repoSlug: "org/web",
  title: "Feature",
  state: "draft",
  url: "https://github.com/org/web/pull/12",
};
const workspace: Workspace = { id: "w1", name: "One", repos: [], linearApiKey: "key" };
const workspaces = [workspace];
const task: Task = {
  id: "task",
  workspaceId: "w1",
  linearIssueId: "issue",
  branchName: "feature",
  createdAt: "2026-10-22",
  members: [
    {
      repoId: "web",
      repoName: "Web",
      localPath: "/web",
      path: "/worktrees/web",
      branchName: "feature",
    },
  ],
};
const tasks = [task];

beforeEach(() => {
  vi.useFakeTimers();
  taskLinearStore.reset();
  resetGithubCache();
  vi.mocked(invoke)
    .mockReset()
    .mockImplementation(async (command) =>
      command === "git_remote_url" ? "git@github.com:org/web.git" : {}
    );
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

it("shares the list request, keeps unmatched PR numbers searchable and survives unmounting", async () => {
  const outside = { ...pr, number: 99, repoSlug: "outside/api" };
  const fetch = vi.spyOn(LinearService.prototype, "fetchIssueLinearInfoBatch").mockResolvedValue({
    issue: { status: null, prs: [pr, outside] },
  });
  const { result, unmount } = renderHook(() => ({
    list: useWorktreeData(tasks, workspace),
    search: useSearchPrNumbers(true, tasks, workspaces),
  }));
  await act(async () => {
    await Promise.resolve();
  });
  expect(fetch).toHaveBeenCalledExactlyOnceWith(["issue"]);
  expect(result.current.search.prNumbersByTask).toEqual({ task: [12, 99] });
  expect(result.current.list.linearInfo.issue.prs).toEqual([pr, outside]);
  expect(vi.getTimerCount()).toBe(1);
  unmount();
  expect(vi.getTimerCount()).toBe(0);
  const remounted = renderHook(() => useSearchPrNumbers(true, tasks, workspaces));
  expect(remounted.result.current.prsByTask.task).toEqual([pr, outside]);
  await act(async () => {
    await Promise.resolve();
  });
  expect(fetch).toHaveBeenCalledOnce();
});

it("hides a previous account's PRs immediately when credentials change", async () => {
  vi.spyOn(LinearService.prototype, "fetchIssueLinearInfoBatch")
    .mockResolvedValueOnce({ issue: { status: null, prs: [pr] } })
    .mockResolvedValue({});
  const { result, rerender } = renderHook(
    ({ current }) => useSearchPrNumbers(true, tasks, current),
    { initialProps: { current: workspaces } }
  );
  await act(async () => {
    await Promise.resolve();
  });
  expect(result.current.prNumbersByTask.task).toEqual([12]);
  rerender({ current: [{ ...workspace, linearApiKey: "different-key" }] });
  expect(result.current.prNumbersByTask.task).toEqual([]);
  await act(async () => {
    await Promise.resolve();
  });
  expect(result.current.prNumbersByTask.task).toEqual([]);
});
