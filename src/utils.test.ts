import { describe, it, expect, vi, afterEach } from "vitest";
import {
  timeAgo,
  migrateLegacyToWorkspaces,
  normalizeWorkspaces,
  normalizeTasks,
} from "./utils";

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

describe("migrateLegacyToWorkspaces", () => {
  it("returns empty for empty/undefined input", () => {
    expect(migrateLegacyToWorkspaces([], [])).toEqual({ workspaces: [], tasks: [] });
    expect(migrateLegacyToWorkspaces(undefined, undefined)).toEqual({
      workspaces: [],
      tasks: [],
    });
  });

  it("turns each legacy repo into a single-member workspace, preserving ids", () => {
    const { workspaces } = migrateLegacyToWorkspaces(
      [{ id: "r1", name: "app", localPath: "/code/app", worktreeBasePath: "/wt/app" }],
      [],
    );
    expect(workspaces).toHaveLength(1);
    expect(workspaces[0].id).toBe("r1");
    expect(workspaces[0].name).toBe("app");
    expect(workspaces[0].repos).toEqual([
      {
        id: "r1",
        name: "app",
        localPath: "/code/app",
        worktreeBasePath: "/wt/app",
      },
    ]);
  });

  it("turns each legacy worktree into a single-member task tied to its workspace", () => {
    const { tasks } = migrateLegacyToWorkspaces(
      [{ id: "r1", name: "app", localPath: "/code/app", worktreeBasePath: "/wt/app" }],
      [
        {
          id: "w1",
          repoId: "r1",
          branchName: "feature/x",
          path: "/wt/app/feature/x",
          createdAt: "2024-01-01T00:00:00Z",
          linearIssueId: "iss1",
          linearIssueTitle: "Do X",
          linearIssueIdentifier: "ABC-1",
        },
      ],
    );
    expect(tasks).toHaveLength(1);
    expect(tasks[0]).toMatchObject({
      id: "w1",
      workspaceId: "r1",
      branchName: "feature/x",
      linearIssueId: "iss1",
      linearIssueIdentifier: "ABC-1",
      workspaceFilePath: null,
    });
    expect(tasks[0].members).toEqual([
      {
        repoId: "r1",
        repoName: "app",
        localPath: "/code/app",
        path: "/wt/app/feature/x",
        branchName: "feature/x",
      },
    ]);
  });

  it("applies the global Linear key to workspaces when no repo has one", () => {
    const { workspaces } = migrateLegacyToWorkspaces(
      [
        { id: "r1", name: "a", localPath: "/a", worktreeBasePath: "/wt/a" },
        { id: "r2", name: "b", localPath: "/b", worktreeBasePath: "/wt/b" },
      ],
      [],
      "lin_api_global",
    );
    expect(workspaces[0].linearApiKey).toBe("lin_api_global");
    expect(workspaces[1].linearApiKey).toBe("lin_api_global");
  });

  it("does not overwrite an existing per-repo Linear key with the global one", () => {
    const { workspaces } = migrateLegacyToWorkspaces(
      [
        { id: "r1", name: "a", localPath: "/a", worktreeBasePath: "/wt/a", linearApiKey: "keep" },
        { id: "r2", name: "b", localPath: "/b", worktreeBasePath: "/wt/b" },
      ],
      [],
      "lin_api_global",
    );
    expect(workspaces[0].linearApiKey).toBe("keep");
    // global only applied when *every* repo lacks a key, so r2 stays null
    expect(workspaces[1].linearApiKey).toBeNull();
  });
});

describe("normalizeWorkspaces / normalizeTasks", () => {
  it("fills defaults for partial workspace/repo objects", () => {
    const [w] = normalizeWorkspaces([{ id: "w1", repos: [{ id: "r1" }] }]);
    expect(w).toEqual({
      id: "w1",
      name: "",
      linearApiKey: null,
      repos: [{ id: "r1", name: "", localPath: "", worktreeBasePath: "" }],
    });
  });

  it("fills task member defaults", () => {
    const [t] = normalizeTasks([
      {
        id: "t1",
        workspaceId: "w1",
        members: [{ repoId: "r1", path: "/a" }],
      },
    ]);
    expect(t.members[0]).toEqual({
      repoId: "r1",
      repoName: "",
      localPath: "",
      path: "/a",
      branchName: "",
    });
    expect(t.workspaceFilePath).toBeNull();
  });

  it("returns empty arrays for nullish input", () => {
    expect(normalizeWorkspaces(undefined)).toEqual([]);
    expect(normalizeTasks(null)).toEqual([]);
  });
});
