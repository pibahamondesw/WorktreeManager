// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, renderHook } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { LinearService } from "../services/linear";
import { IssueLinearInfo, Task, Workspace } from "../types";
import { resetGithubCache } from "../services/github";
import { useWorktreeData } from "./useWorktreeData";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const pr = {
  repoSlug: "org/repo",
  number: 1,
  state: "open",
  title: "One",
  url: "https://github.com/org/repo/pull/1",
};
const task: Task = {
  id: "t1",
  workspaceId: "w1",
  linearIssueId: "i1",
  members: [],
  branchName: "feat",
  createdAt: "2026-10-07",
};
const tasks = [task];
const workspace: Workspace = { id: "w1", name: "One", repos: [] };
let fetchInfo: ReturnType<typeof vi.fn<LinearService["fetchIssueLinearInfoBatch"]>>;
let linear: LinearService;

beforeEach(() => {
  resetGithubCache();
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-07"));
  vi.mocked(invoke).mockReset().mockResolvedValue({});
  fetchInfo = vi.fn().mockResolvedValue({ i1: { status: null, prs: [pr] } });
  linear = { fetchIssueLinearInfoBatch: fetchInfo } as unknown as LinearService;
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

async function settle() {
  await act(async () => {
    await Promise.resolve();
  });
}

it("refreshes GitHub at five minutes while retaining the fifteen-minute Linear interval", async () => {
  const { result } = renderHook(() => useWorktreeData(tasks, workspace, linear));
  await settle();
  expect(invoke).toHaveBeenCalledOnce();
  await act(() => vi.advanceTimersByTimeAsync(4 * 60 * 1000));
  expect(invoke).toHaveBeenCalledOnce();
  await act(() => vi.advanceTimersByTimeAsync(60 * 1000));
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(fetchInfo).toHaveBeenCalledOnce();
  await act(() => result.current.handleRefresh());
  expect(invoke).toHaveBeenCalledTimes(3);
  expect(fetchInfo).toHaveBeenCalledTimes(2);
});

it("pauses while hidden and refreshes on visibility when due", async () => {
  renderHook(() => useWorktreeData(tasks, workspace, linear));
  await settle();
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
  await act(() => vi.advanceTimersByTimeAsync(6 * 60 * 1000));
  expect(invoke).toHaveBeenCalledOnce();
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
  await act(async () => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
  expect(invoke).toHaveBeenCalledTimes(2);
});

it("refreshes on workspace switch and ignores delayed Linear results from the old workspace", async () => {
  let resolve!: (info: Record<string, IssueLinearInfo>) => void;
  fetchInfo.mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      })
  );
  const otherTask = { ...task, id: "t2", workspaceId: "w2", linearIssueId: "i2" };
  const otherTasks = [otherTask];
  const otherWorkspace = { ...workspace, id: "w2" };
  fetchInfo.mockResolvedValue({ i2: { status: null, prs: [{ ...pr, repoSlug: "other/repo" }] } });
  const { rerender } = renderHook(
    ({ currentTasks, currentWorkspace }) => useWorktreeData(currentTasks, currentWorkspace, linear),
    { initialProps: { currentTasks: tasks, currentWorkspace: workspace } }
  );
  rerender({ currentTasks: otherTasks, currentWorkspace: otherWorkspace });
  await settle();
  await act(async () => {
    resolve({ i1: { status: null, prs: [pr] } });
  });
  await act(() => vi.advanceTimersByTimeAsync(5 * 60 * 1000));
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(invoke).toHaveBeenLastCalledWith("github_pr_status_batch", {
    repoSlug: "other/repo",
    prNumbers: [1],
  });
});

it("notifies workspace readiness even if Linear fails", async () => {
  fetchInfo.mockRejectedValue(new Error("offline"));
  const onReady = vi.fn();
  renderHook(() => useWorktreeData(tasks, workspace, linear, onReady));
  await settle();
  expect(onReady).toHaveBeenCalledWith("w1");
  expect(invoke).not.toHaveBeenCalled();
});

it("refreshes known PRs manually even when Linear is unavailable", async () => {
  const { result } = renderHook(() => useWorktreeData(tasks, workspace, linear));
  await settle();
  fetchInfo.mockRejectedValue(new Error("offline"));
  await act(() => result.current.handleRefresh());
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(invoke).toHaveBeenLastCalledWith("github_pr_status_batch", {
    repoSlug: "org/repo",
    prNumbers: [1],
  });
});
