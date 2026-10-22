// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { IssueLinearInfo, PullRequestInfo, Task, Workspace } from "../types";
import { LINEAR_REFRESH_INTERVAL_MS, TaskLinearStore } from "./taskLinear";

const pr: PullRequestInfo = {
  number: 12,
  repoSlug: "Org/Web",
  title: "Feature",
  state: "open",
  url: "https://github.com/org/web/pull/12",
};
const info: IssueLinearInfo = {
  status: { name: "In Progress", type: "started" },
  project: { id: "project", name: "API" },
  prs: [pr],
};
const workspace: Workspace = { id: "w1", name: "One", repos: [], linearApiKey: "key" };
const member = (repoId: string, localPath = `/${repoId}`) => ({
  repoId,
  localPath,
  repoName: repoId,
  path: `/worktrees/${repoId}`,
  branchName: "feature",
});
const task: Task = {
  id: "t1",
  workspaceId: "w1",
  linearIssueId: "i1",
  members: [member("web")],
  branchName: "feature",
  createdAt: "2026-10-22",
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function fixture() {
  const fetch = vi
    .fn<(ids: string[]) => Promise<Record<string, IssueLinearInfo>>>()
    .mockImplementation(async (ids) => Object.fromEntries(ids.map((id) => [id, info])));
  const linear = vi.fn(() => ({ fetchIssueLinearInfoBatch: fetch }));
  const resolveSlug = vi.fn(async (path: string) => (path === "/gitlab" ? null : `org${path}`));
  const refreshPrs = vi.fn().mockResolvedValue(undefined);
  const store = new TaskLinearStore({ linear, resolveSlug, refreshPrs });
  return { store, fetch, linear, resolveSlug, refreshPrs };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-22"));
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
});
afterEach(() => vi.useRealTimers());

it("batches and deduplicates overlapping queries across workspaces with the same credentials", async () => {
  const { store, fetch } = fixture();
  const other = { ...workspace, id: "w2" };
  const first = store.refreshIssues(workspace, ["i1", "i1"]);
  const second = store.refreshIssues(other, ["i1", "i2"]);
  const [one, two] = await Promise.all([first, second]);
  expect(fetch).toHaveBeenCalledExactlyOnceWith(["i1", "i2"]);
  expect(one).toEqual({ i1: info });
  expect(two).toEqual({ i1: info, i2: info });
  expect(store.info(other)).toEqual({ i1: info, i2: info });
});

it("shares requests already in flight while fetching additional issues", async () => {
  const { store, fetch } = fixture();
  const pending = deferred<Record<string, IssueLinearInfo>>();
  fetch.mockReturnValueOnce(pending.promise);
  const first = store.refreshIssues(workspace, ["i1"]);
  await Promise.resolve();
  const second = store.refreshIssues(workspace, ["i1", "i2"]);
  await Promise.resolve();
  pending.resolve({ i1: info });
  await Promise.all([first, second]);
  expect(fetch.mock.calls).toEqual([[["i1"]], [["i2"]]]);
});

it("keeps cache across subscriptions and only refetches stale or explicitly refreshed issues", async () => {
  const { store, fetch } = fixture();
  const unsubscribe = store.subscribe(vi.fn());
  await store.refreshIssues(workspace, ["i1"]);
  unsubscribe();
  await store.refreshIssues(workspace, ["i1"]);
  expect(fetch).toHaveBeenCalledOnce();
  await store.refreshIssues(workspace, ["i1"], true);
  expect(fetch).toHaveBeenCalledTimes(2);
  vi.advanceTimersByTime(LINEAR_REFRESH_INTERVAL_MS);
  await store.refreshIssues(workspace, ["i1"]);
  expect(fetch).toHaveBeenCalledTimes(3);
});

it("isolates credentials and prevents an old response from replacing current account data", async () => {
  const { store, fetch } = fixture();
  const pending = deferred<Record<string, IssueLinearInfo>>();
  fetch.mockReturnValueOnce(pending.promise);
  const old = store.refreshIssues(workspace, ["i1"]);
  await Promise.resolve();
  const current = { ...workspace, linearApiKey: "new-key" };
  expect(store.taskInfo(task, current)).toBeUndefined();
  await store.refreshIssues(current, ["i1"]);
  pending.resolve({ i1: { status: null, prs: [] } });
  await old;
  expect(store.taskInfo(task, current)).toEqual(info);
  expect(store.taskInfo(task, { ...current, linearApiKey: null })).toBeUndefined();
});

it("merges partial responses and retains known PRs during a failed manual refresh", async () => {
  const { store, fetch, refreshPrs } = fixture();
  await store.refreshIssues(workspace, ["i1", "i2"]);
  fetch.mockResolvedValueOnce({ i2: { status: null, prs: [] } });
  expect(await store.refreshIssues(workspace, ["i1", "i2"], true)).toEqual({
    i2: { status: null, prs: [] },
  });
  expect(store.taskInfo(task, workspace)).toEqual(info);
  fetch.mockRejectedValueOnce(new Error("offline"));
  await expect(store.refreshWorkspace(workspace, [task], true)).rejects.toThrow("offline");
  expect(refreshPrs).toHaveBeenLastCalledWith([pr], true);
  await store.refreshIssues(workspace, ["i1"], true);
  expect(store.taskInfo(task, workspace)).toEqual(info);
});

it("ignores pending results after clearing the cache", async () => {
  const { store, fetch } = fixture();
  const pending = deferred<Record<string, IssueLinearInfo>>();
  fetch.mockReturnValueOnce(pending.promise);
  const request = store.refreshIssues(workspace, ["i1"]);
  await Promise.resolve();
  store.reset();
  pending.resolve({ i1: info });
  await request;
  expect(store.info(workspace)).toEqual({});
});

describe("repository matching", () => {
  it("uses the injected Linear port and slug resolver for members and preserves unmatched PRs", async () => {
    const { store, fetch, resolveSlug } = fixture();
    const outside = { ...pr, number: 13, repoSlug: "org/api" };
    const fork = { ...pr, number: 14, repoSlug: "fork/web" };
    fetch.mockResolvedValue({ i1: { ...info, prs: [pr, outside, fork] } });
    await store.refreshWorkspace(workspace, [task]);
    expect(store.taskPrs(task, workspace).unmatched.size).toBe(0);
    await Promise.all([store.resolveSlug("/web"), store.resolveSlug("/web")]);
    expect(resolveSlug).toHaveBeenCalledExactlyOnceWith("/web");
    const grouped = store.taskPrs(task, workspace);
    expect(grouped.byMember).toEqual({ web: [pr] });
    expect([...grouped.unmatched]).toEqual([
      ["org/api", [outside]],
      ["fork/web", [fork]],
    ]);
    await store.resolveSlug("/web");
    expect(resolveSlug).toHaveBeenCalledOnce();
  });

  it("matches every participating repository and leaves nonmembers unmatched", async () => {
    const { store, fetch } = fixture();
    const api = { ...pr, repoSlug: "ORG/API", number: 99 };
    fetch.mockResolvedValue({ i1: { ...info, prs: [pr, api] } });
    await store.refreshIssues(workspace, ["i1"]);
    await Promise.all([store.resolveSlug("/web"), store.resolveSlug("/api")]);
    expect(
      store.taskPrs({ ...task, members: [member("web"), member("api")] }, workspace).byMember
    ).toEqual({ web: [pr], api: [api] });
    expect([...store.taskPrs(task, workspace).unmatched.values()]).toEqual([[api]]);
  });

  it("handles non-GitHub and failed remotes without leaving matching in a loading state", async () => {
    const { store, resolveSlug } = fixture();
    resolveSlug.mockRejectedValueOnce(new Error("missing repository"));
    await store.resolveSlug("/web");
    await store.resolveSlug("/gitlab");
    expect(store.repoSlugs([member("web"), member("gitlab")])).toEqual({});
  });
});

it("uses one timer for all consumers, pauses while hidden and stops after the last consumer leaves", async () => {
  const { store, fetch, refreshPrs } = fixture();
  const onRefresh = vi.fn();
  await store.refreshWorkspace(workspace, [task]);
  const stopOne = store.watch([task], [workspace], onRefresh);
  const stopTwo = store.watch([task], [workspace]);
  expect(vi.getTimerCount()).toBe(1);
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
  await vi.advanceTimersByTimeAsync(LINEAR_REFRESH_INTERVAL_MS);
  expect(fetch).toHaveBeenCalledOnce();
  expect(refreshPrs).toHaveBeenCalledOnce();
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
  document.dispatchEvent(new Event("visibilitychange"));
  await vi.advanceTimersByTimeAsync(0);
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(onRefresh).toHaveBeenCalledOnce();
  stopOne();
  expect(vi.getTimerCount()).toBe(1);
  stopTwo();
  expect(vi.getTimerCount()).toBe(0);
  document.dispatchEvent(new Event("visibilitychange"));
  await vi.advanceTimersByTimeAsync(LINEAR_REFRESH_INTERVAL_MS);
  expect(fetch).toHaveBeenCalledTimes(2);
});

it("retains a successful Linear refresh if the auxiliary GitHub refresh fails", async () => {
  const { store, refreshPrs } = fixture();
  refreshPrs.mockRejectedValueOnce(new Error("GitHub unavailable"));
  await expect(store.refreshWorkspace(workspace, [task])).resolves.toEqual({ i1: info });
  expect(store.taskInfo(task, workspace)).toEqual(info);
});

it("retries an empty Linear response instead of caching a network failure as fresh", async () => {
  const { store, fetch } = fixture();
  fetch.mockResolvedValueOnce({});
  await store.refreshIssues(workspace, ["i1"]);
  expect(store.taskInfo(task, workspace)).toBeUndefined();
  await store.refreshIssues(workspace, ["i1"]);
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(store.taskInfo(task, workspace)).toEqual(info);
});

it("only renews returned issues when a stale batch receives a partial response", async () => {
  const { store, fetch } = fixture();
  await store.refreshIssues(workspace, ["i1", "i2"]);
  vi.advanceTimersByTime(LINEAR_REFRESH_INTERVAL_MS);
  fetch.mockResolvedValueOnce({ i1: info });
  await store.refreshIssues(workspace, ["i1", "i2"]);
  expect(store.info(workspace)).toEqual({ i1: info, i2: info });
  await store.refreshIssues(workspace, ["i1", "i2"]);
  expect(fetch).toHaveBeenCalledTimes(3);
  expect(fetch).toHaveBeenLastCalledWith(["i2"]);
});
