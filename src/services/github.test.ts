import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { GithubPrStatus, PullRequestInfo } from "../types";
import { githubPrStatus, invalidateGithubRepo, refreshGithubPrs, resetGithubCache } from "./github";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const pr: PullRequestInfo = {
  repoSlug: "Org/Repo",
  number: 1,
  state: "draft",
  title: "One",
  url: "https://github.com/Org/Repo/pull/1",
};
const status: GithubPrStatus = { state: "open", isDraft: true, ci: "passing", review: "approved" };

beforeEach(() => {
  resetGithubCache();
  vi.mocked(invoke).mockReset().mockResolvedValue({ 1: status });
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-07"));
});
afterEach(() => vi.useRealTimers());

describe("GitHub cache", () => {
  it("batches and deduplicates each repo, including PRs without a local clone", async () => {
    await refreshGithubPrs([pr, pr, { ...pr, number: 2 }, { ...pr, repoSlug: "other/repo" }]);
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke).toHaveBeenCalledWith("github_pr_status_batch", {
      repoSlug: "org/repo",
      prNumbers: [1, 2],
    });
    expect(githubPrStatus(pr)).toEqual(status);
  });

  it("reuses fresh values for five minutes and supports forced refresh", async () => {
    await refreshGithubPrs([pr]);
    await refreshGithubPrs([pr]);
    expect(invoke).toHaveBeenCalledOnce();
    vi.advanceTimersByTime(5 * 60 * 1000);
    await refreshGithubPrs([pr]);
    await refreshGithubPrs([pr], true);
    expect(invoke).toHaveBeenCalledTimes(3);
  });

  it("shares overlapping requests and fetches only missing numbers", async () => {
    let resolve!: (value: unknown) => void;
    vi.mocked(invoke).mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        })
    );
    const first = refreshGithubPrs([pr]);
    const second = refreshGithubPrs([pr, { ...pr, number: 2 }]);
    expect(invoke).toHaveBeenCalledOnce();
    resolve({ 1: status });
    await Promise.all([first, second]);
    expect(invoke).toHaveBeenLastCalledWith("github_pr_status_batch", {
      repoSlug: "org/repo",
      prNumbers: [2],
    });
  });

  it("discards a response started before ready invalidation", async () => {
    await refreshGithubPrs([pr]);
    let resolve!: (value: unknown) => void;
    vi.mocked(invoke).mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        })
    );
    const pending = refreshGithubPrs([pr], true);
    invalidateGithubRepo(pr.repoSlug, pr);
    expect(githubPrStatus(pr)?.isDraft).toBe(false);
    resolve({ 1: status });
    await pending;
    expect(githubPrStatus(pr)?.isDraft).toBe(false);
  });

  it("falls back for failed or missing PRs while preserving successful peers", async () => {
    await refreshGithubPrs([pr]);
    vi.mocked(invoke).mockRejectedValueOnce(new Error("offline"));
    await refreshGithubPrs([pr], true);
    expect(githubPrStatus(pr)).toBeUndefined();
    vi.mocked(invoke).mockResolvedValueOnce({ 1: status });
    await refreshGithubPrs([pr, { ...pr, number: 2 }], true);
    expect(githubPrStatus(pr)).toEqual(status);
    expect(githubPrStatus({ ...pr, number: 2 })).toBeUndefined();
  });
});
