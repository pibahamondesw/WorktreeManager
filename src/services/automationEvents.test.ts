import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { GithubPrStatus, PullRequestInfo, Task } from "../types";
import {
  agentAppEvent,
  emitAppEvent,
  PrEventTracker,
  prActions,
  reportPrChanges,
  taskAppEvent,
} from "./automationEvents";
import { githubPrStatus } from "./github";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("./github", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./github")>()),
  githubPrStatus: vi.fn(),
}));

const task: Task = {
  id: "t1",
  workspaceId: "w1",
  branchName: "feature",
  createdAt: "2026-11-10",
  members: [],
  automationId: "reviewer",
  linearIssueIdentifier: "ENG-1",
};
const pr: PullRequestInfo = {
  number: 7,
  repoSlug: "acme/api",
  title: "Feature",
  state: "open",
  url: "https://github.com/acme/api/pull/7",
};
const open: GithubPrStatus = {
  state: "open",
  isDraft: true,
  ci: "running",
  review: "pending",
  headSha: "a1",
};

describe("automation app events", () => {
  it("hands events to the engine and swallows its failures", async () => {
    vi.mocked(invoke).mockRejectedValueOnce(new Error("no engine"));
    const event = taskAppEvent("task.deleted", { ...task, automationId: undefined });
    emitAppEvent(event);
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("automation_app_event", {
        event: {
          type: "task.deleted",
          task: { id: "t1", workspaceId: "w1", branch: "feature", linearIssue: "ENG-1" },
        },
      })
    );
  });

  it("describes agent turns", () => {
    const turn = { state: "waiting" as const, agent: "claude", surface: "chat", at: 5 };
    expect(agentAppEvent(task, turn)).toEqual({
      type: "agent.waiting",
      task: {
        id: "t1",
        workspaceId: "w1",
        branch: "feature",
        automationId: "reviewer",
        linearIssue: "ENG-1",
      },
      agent: { id: "claude", surface: "chat", at: 5 },
    });
  });

  it("derives pull request actions from two statuses", () => {
    expect(prActions(open, { ...open, isDraft: false })).toEqual(["ready"]);
    expect(prActions(open, { ...open, state: "merged", ci: "passing" })).toEqual([
      "merged",
      "checks_passed",
    ]);
    expect(prActions({ ...open, state: "merged" }, { ...open, state: "closed" })).toEqual([
      "closed",
    ]);
    expect(prActions({ ...open, state: "closed" }, open)).toEqual(["reopened"]);
    const failing = { ...open, ci: "failing" as const };
    expect(prActions(failing, failing)).toEqual([]);
    expect(prActions(failing, { ...failing, headSha: "b2" })).toEqual(["checks_failed"]);
    expect(prActions(open, { ...open, review: "approved" })).toEqual(["approved"]);
    expect(prActions(open, { ...open, review: "changes_requested" })).toEqual([
      "changes_requested",
    ]);
  });

  it("reports pull request changes after a baseline, per task", () => {
    const emit = vi.fn();
    const tracker = new PrEventTracker(emit);
    tracker.observe(task, pr, undefined);
    tracker.observe(task, pr, { ...open, headSha: undefined });
    tracker.observe(task, pr, open);
    expect(emit).not.toHaveBeenCalled();

    tracker.observe(task, pr, { ...open, state: "merged" });
    tracker.observe({ ...task, id: "t2" }, pr, { ...open, state: "merged" });

    expect(emit).toHaveBeenCalledOnce();
    expect(emit).toHaveBeenCalledWith({
      type: "pr.merged",
      task: {
        id: "t1",
        workspaceId: "w1",
        branch: "feature",
        automationId: "reviewer",
        linearIssue: "ENG-1",
      },
      pr: {
        state: "merged",
        isDraft: true,
        ci: "running",
        review: "pending",
        headSha: "a1",
        repo: "acme/api",
        number: 7,
        url: "https://github.com/acme/api/pull/7",
      },
    });
  });

  it("reads the cached GitHub status of every task pull request", () => {
    reportPrChanges([{ task, prs: [pr] }]);
    expect(githubPrStatus).toHaveBeenCalledWith(pr);
  });
});
