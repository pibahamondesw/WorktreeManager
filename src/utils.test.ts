import { describe, it, expect, vi, afterEach } from "vitest";
import { timeAgo, migrateRepos } from "./utils";

describe("timeAgo", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  function epochSecondsAgo(seconds: number): number {
    return Date.now() / 1000 - seconds;
  }

  it("returns empty label for epoch 0", () => {
    const result = timeAgo(0);
    expect(result).toEqual({ label: "", stale: false, veryStale: false });
  });

  it('returns "just now" for very recent timestamps', () => {
    const result = timeAgo(epochSecondsAgo(10));
    expect(result.label).toBe("just now");
    expect(result.stale).toBe(false);
    expect(result.veryStale).toBe(false);
  });

  it("returns minutes for timestamps under an hour", () => {
    const result = timeAgo(epochSecondsAgo(60 * 15));
    expect(result.label).toBe("15m ago");
    expect(result.stale).toBe(false);
  });

  it("returns hours for timestamps under a day", () => {
    const result = timeAgo(epochSecondsAgo(3600 * 5));
    expect(result.label).toBe("5h ago");
    expect(result.stale).toBe(false);
  });

  it("returns days for timestamps under a month", () => {
    const result = timeAgo(epochSecondsAgo(86400 * 2));
    expect(result.label).toBe("2d ago");
    expect(result.stale).toBe(false);
    expect(result.veryStale).toBe(false);
  });

  it("marks 3-6 day old timestamps as stale", () => {
    const result = timeAgo(epochSecondsAgo(86400 * 4));
    expect(result.label).toBe("4d ago");
    expect(result.stale).toBe(true);
    expect(result.veryStale).toBe(false);
  });

  it("marks 7+ day old timestamps as veryStale", () => {
    const result = timeAgo(epochSecondsAgo(86400 * 10));
    expect(result.label).toBe("10d ago");
    expect(result.stale).toBe(false);
    expect(result.veryStale).toBe(true);
  });

  it("returns months for timestamps over 30 days", () => {
    const result = timeAgo(epochSecondsAgo(86400 * 65));
    expect(result.label).toBe("2mo ago");
    expect(result.veryStale).toBe(true);
  });
});

describe("migrateRepos", () => {
  it("returns empty array for empty input", () => {
    expect(migrateRepos([])).toEqual([]);
  });

  it("passes through complete repo objects and adds linearApiKey null", () => {
    const repos = [
      { id: "1", name: "app", localPath: "/code/app", worktreeBasePath: "/wt/app" },
    ];
    expect(migrateRepos(repos)).toEqual([
      { id: "1", name: "app", localPath: "/code/app", worktreeBasePath: "/wt/app", linearApiKey: null },
    ]);
  });

  it("fills in missing localPath with empty string", () => {
    const repos = [{ id: "1", name: "app", worktreeBasePath: "/wt/app" }];
    const result = migrateRepos(repos);
    expect(result[0].localPath).toBe("");
  });

  it("fills in missing worktreeBasePath with empty string", () => {
    const repos = [{ id: "1", name: "app", localPath: "/code/app" }];
    const result = migrateRepos(repos);
    expect(result[0].worktreeBasePath).toBe("");
  });

  it("fills in both missing fields for very old data", () => {
    const repos = [{ id: "1", name: "legacy" }];
    const result = migrateRepos(repos);
    expect(result[0]).toEqual({
      id: "1",
      name: "legacy",
      localPath: "",
      worktreeBasePath: "",
      linearApiKey: null,
    });
  });

  it("handles multiple repos with mixed completeness", () => {
    const repos = [
      { id: "1", name: "complete", localPath: "/a", worktreeBasePath: "/b" },
      { id: "2", name: "partial", localPath: "/c" },
      { id: "3", name: "minimal" },
    ];
    const result = migrateRepos(repos);
    expect(result).toHaveLength(3);
    expect(result[1].worktreeBasePath).toBe("");
    expect(result[2].localPath).toBe("");
    expect(result[2].worktreeBasePath).toBe("");
  });

  it("copies global Linear API key to all repos when none have one", () => {
    const repos = [
      { id: "1", name: "app1", localPath: "/a", worktreeBasePath: "/wt/a" },
      { id: "2", name: "app2", localPath: "/b", worktreeBasePath: "/wt/b" },
    ];
    const result = migrateRepos(repos, "lin_api_global");
    expect(result[0].linearApiKey).toBe("lin_api_global");
    expect(result[1].linearApiKey).toBe("lin_api_global");
  });

  it("does not overwrite existing per-repo Linear API key with global key", () => {
    const repos = [
      { id: "1", name: "app1", localPath: "/a", worktreeBasePath: "/wt/a", linearApiKey: "lin_api_existing" },
      { id: "2", name: "app2", localPath: "/b", worktreeBasePath: "/wt/b" },
    ];
    const result = migrateRepos(repos, "lin_api_global");
    expect(result[0].linearApiKey).toBe("lin_api_existing");
    expect(result[1].linearApiKey).toBeNull();
  });

  it("does not apply global key when no repos exist", () => {
    const result = migrateRepos([], "lin_api_global");
    expect(result).toEqual([]);
  });

  it("does not apply global key when it is null", () => {
    const repos = [
      { id: "1", name: "app1", localPath: "/a", worktreeBasePath: "/wt/a" },
    ];
    const result = migrateRepos(repos, null);
    expect(result[0].linearApiKey).toBeNull();
  });
});
