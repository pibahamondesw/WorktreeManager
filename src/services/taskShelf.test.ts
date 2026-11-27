import { afterEach, describe, expect, it, vi } from "vitest";
import { GitStatus, GithubPrStatus, PullRequestInfo, Task } from "../types";
import {
  AutoShelveSettings,
  AutoShelveSignals,
  autoShelveReason,
  autoShelveSignals,
  TaskSignalSources,
} from "./taskShelf";
import { githubPrStatus } from "./github";

vi.mock("./github", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./github")>()),
  githubPrStatus: vi.fn(),
}));

const DAY = 24 * 60 * 60 * 1000;
const NOW = Date.parse("2026-03-01T00:00:00.000Z");
const SETTINGS: AutoShelveSettings = { inactiveDays: 30, since: "2026-01-01T00:00:00.000Z" };
const TASK: Task = {
  id: "t1",
  workspaceId: "w1",
  branchName: "feature",
  linearIssueId: "issue",
  members: [
    { repoId: "api", repoName: "api", localPath: "/api", path: "/wt/api", branchName: "feature" },
    { repoId: "web", repoName: "web", localPath: "/web", path: "/wt/web", branchName: "feature" },
  ],
  createdAt: "2026-01-01T00:00:00.000Z",
};
const RECENT = NOW - DAY;

function reason(signals: AutoShelveSignals, task: Partial<Task> = {}, settings = SETTINGS) {
  return autoShelveReason({ ...TASK, ...task }, signals, NOW, settings);
}

afterEach(() => vi.mocked(githubPrStatus).mockReset());

describe("autoShelveReason", () => {
  it.each(["completed", "canceled"])("moves a task whose Linear issue is %s", (type) => {
    expect(reason({ linearStateType: type, lastCommitAt: RECENT })).toBe("linear");
  });

  it("keeps a task whose Linear issue is still in progress", () => {
    expect(reason({ linearStateType: "started", lastCommitAt: RECENT })).toBeNull();
  });

  it("moves a task whose pull requests are all merged or closed", () => {
    expect(reason({ prStates: ["merged", "closed"], lastCommitAt: RECENT })).toBe("pr");
  });

  it("keeps a task with an open pull request or none", () => {
    expect(reason({ prStates: ["merged", "open"], lastCommitAt: RECENT })).toBeNull();
    expect(reason({ prStates: [], lastCommitAt: RECENT })).toBeNull();
  });

  it("moves a task with no activity for the configured days", () => {
    expect(reason({ lastCommitAt: NOW - 30 * DAY })).toBe("inactive");
    expect(reason({ lastCommitAt: NOW - 29 * DAY })).toBeNull();
  });

  it("counts the last recorded visit or agent event as activity", () => {
    const lastActiveAt = new Date(RECENT).toISOString();
    expect(reason({ lastCommitAt: NOW - 40 * DAY }, { lastActiveAt })).toBeNull();
  });

  it("does not count older activity than when the rule started", () => {
    const since = new Date(NOW - 10 * DAY).toISOString();
    expect(reason({ lastCommitAt: NOW - 90 * DAY }, {}, { ...SETTINGS, since })).toBeNull();
  });

  it("waits for git status and a start date before judging inactivity", () => {
    expect(reason({})).toBeNull();
    expect(reason({ lastCommitAt: NOW - 90 * DAY }, {}, { ...SETTINGS, since: null })).toBeNull();
  });

  it("does not judge inactivity when the rule is off", () => {
    expect(
      reason({ lastCommitAt: NOW - 90 * DAY }, {}, { ...SETTINGS, inactiveDays: null })
    ).toBeNull();
  });

  it.each([
    ["already finished", { shelvedAt: "2026-02-01T00:00:00.000Z" }],
    ["snoozed", { snoozedUntil: "2026-04-01T00:00:00.000Z" }],
    ["pinned", { pinOrder: 0 }],
    ["set to never move", { autoShelve: false as const }],
  ])("leaves a task that is %s", (_label, task) => {
    expect(reason({ linearStateType: "completed" }, task)).toBeNull();
  });
});

describe("autoShelveSignals", () => {
  const pr = (repoSlug: string, number: number): PullRequestInfo =>
    ({ repoSlug, number, url: "", title: "", state: "open" }) as PullRequestInfo;
  const status = (epoch: number) => ({ last_commit_epoch: epoch }) as GitStatus;

  function sources(overrides: Partial<TaskSignalSources> = {}): TaskSignalSources {
    return {
      linearInfo: {
        issue: {
          status: { name: "Done", type: "completed" },
          prs: [pr("acme/api", 1), pr("acme/web", 2), pr("acme/other", 3)],
        },
      },
      gitStatuses: { "/wt/api": status(100), "/wt/web": status(300) },
      repoSlugs: () => ({ api: "acme/api", web: "acme/web" }),
      ...overrides,
    };
  }

  it("reads the Linear state, the member PR states and the newest commit", () => {
    vi.mocked(githubPrStatus).mockImplementation(
      (item) => ({ state: item.number === 1 ? "merged" : "closed" }) as GithubPrStatus
    );
    expect(autoShelveSignals(TASK, sources())).toEqual({
      linearStateType: "completed",
      prStates: ["merged", "closed"],
      lastCommitAt: 300_000,
    });
  });

  it("leaves PR states unknown until every status and repository is known", () => {
    vi.mocked(githubPrStatus).mockReturnValue(undefined);
    expect(autoShelveSignals(TASK, sources()).prStates).toBeUndefined();
    expect(autoShelveSignals(TASK, sources({ repoSlugs: () => null })).prStates).toBeUndefined();
  });

  it("leaves PR states unknown while a member repository has no pull request", () => {
    vi.mocked(githubPrStatus).mockReturnValue({ state: "merged" } as GithubPrStatus);
    const info = { status: null, prs: [pr("acme/api", 1)] };
    expect(
      autoShelveSignals(TASK, sources({ linearInfo: { issue: info } })).prStates
    ).toBeUndefined();
  });

  it("leaves Linear and PR signals unknown until the issue loads", () => {
    const signals = autoShelveSignals(TASK, sources({ linearInfo: {} }));
    expect(signals.linearStateType).toBeUndefined();
    expect(signals.prStates).toBeUndefined();
  });

  it("leaves the last commit unknown until every member reports git status", () => {
    const signals = autoShelveSignals(TASK, sources({ gitStatuses: { "/wt/api": status(100) } }));
    expect(signals.lastCommitAt).toBeUndefined();
  });
});
