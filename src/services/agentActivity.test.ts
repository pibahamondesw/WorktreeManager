import { describe, expect, it } from "vitest";
import { Task } from "../types";
import {
  AgentActivities,
  AgentActivity,
  DEFAULT_AGENT_ALERTS,
  alertsFor,
  applyAgentEvent,
  chatActivityEvent,
  embeddedSurface,
  endStaleSessions,
  markTaskSeen,
  nextActivity,
  normalizeAgentAlerts,
  notificationFor,
  pruneActivities,
  strongestIndicator,
  taskForCwd,
  taskIndicator,
  unreadCount,
} from "./agentActivity";
import { ChatInfo } from "./chat";

function task(id: string, paths: string[], extra: Partial<Task> = {}): Task {
  return {
    id,
    workspaceId: "w1",
    branchName: `branch-${id}`,
    createdAt: "2026-09-24T00:00:00Z",
    members: paths.map((path, i) => ({
      repoId: `r${i}`,
      repoName: `repo${i}`,
      localPath: `/repos/repo${i}`,
      path,
      branchName: `branch-${id}`,
    })),
    ...extra,
  };
}

const activity = (overrides: Partial<AgentActivity>): AgentActivity => ({
  state: "working",
  agent: "claude",
  surface: "terminal",
  unread: false,
  at: 1,
  ...overrides,
});

const event = (
  state: AgentActivity["state"],
  at: number,
  surface: AgentActivity["surface"] = "terminal"
) => ({
  state,
  agent: "claude" as const,
  surface,
  at,
});

describe("taskForCwd", () => {
  const tasks = [
    task("outer", ["/wt/app/feature"]),
    task("multi", ["/wt/api/multi", "/wt/web/multi"]),
    task("nested", ["/wt/app/feature/nested"]),
  ];

  it("matches any member worktree and its subdirectories", () => {
    expect(taskForCwd(tasks, "/wt/web/multi")?.id).toBe("multi");
    expect(taskForCwd(tasks, "/wt/api/multi/src/lib")?.id).toBe("multi");
  });

  it("prefers the deepest worktree and respects path boundaries", () => {
    expect(taskForCwd(tasks, "/wt/app/feature/nested/src")?.id).toBe("nested");
    expect(taskForCwd(tasks, "/wt/app/feature-2")).toBeUndefined();
    expect(taskForCwd(tasks, "/elsewhere")).toBeUndefined();
  });
});

describe("activity lifecycle", () => {
  it("keeps results unread until the task is seen, then drops a read done", () => {
    let activities: AgentActivities = {};
    activities = applyAgentEvent(activities, "t1", event("working", 1), false);
    expect(activities.t1).toMatchObject({ state: "working", unread: false });

    activities = applyAgentEvent(activities, "t1", event("done", 2), false);
    activities = applyAgentEvent(activities, "t2", event("waiting", 2), false);
    expect(unreadCount(activities)).toBe(2);

    activities = markTaskSeen(activities, "t1");
    expect(activities.t1).toBeUndefined();
    activities = markTaskSeen(activities, "t2");
    expect(activities.t2).toMatchObject({ state: "waiting", unread: false });
    expect(markTaskSeen(activities, "t2")).toBe(activities);
  });

  it("ignores events older than the one already applied", () => {
    const done = applyAgentEvent({}, "t1", event("done", 20), false);
    expect(applyAgentEvent(done, "t1", event("working", 10), false)).toBe(done);
    expect(applyAgentEvent(done, "t1", event("working", 30), false).t1.state).toBe("working");
  });

  it("keeps an async question waiting until the user replies, alerting once", () => {
    const asked = nextActivity(
      undefined,
      { ...event("waiting", 1, "external"), asyncQuestion: true },
      false
    )!;
    expect(asked).toMatchObject({ state: "waiting", openQuestion: true, unread: true });
    expect(alertsFor(undefined, asked)).toBe("waiting");

    const working = nextActivity(asked, event("working", 2, "external"), false)!;
    const finished = nextActivity(working, event("done", 3, "external"), false)!;
    expect(finished).toMatchObject({ state: "waiting", openQuestion: true, at: 3 });
    expect(alertsFor(working, finished)).toBeNull();

    const replied = nextActivity(
      finished,
      { ...event("working", 4, "external"), prompt: true },
      false
    )!;
    expect(replied).toEqual(expect.objectContaining({ state: "working", unread: false }));
    expect(replied.openQuestion).toBeUndefined();
    expect(alertsFor(finished, replied)).toBeNull();
    expect(alertsFor(replied, nextActivity(replied, event("done", 5), false))).toBe("done");
  });

  it("does not mark results unread for the task being viewed", () => {
    expect(applyAgentEvent({}, "t1", event("done", 1), true).t1.unread).toBe(false);
  });

  it("clears work of embedded sessions that are gone and forgets deleted tasks", () => {
    const activities: AgentActivities = {
      editor: activity({ surface: "editor" }),
      closedChat: activity({ surface: "chat", state: "waiting" }),
      exitedTerminal: activity({ surface: "terminal" }),
      unlistedTerminal: activity({ surface: "terminal" }),
      external: activity({ surface: "external" }),
      finished: activity({ surface: "editor", state: "done", unread: true }),
    };
    const next = endStaleSessions(activities, {
      editor: { kind: "running" },
      exitedTerminal: { kind: "exited", code: 0 },
    });
    expect(Object.keys(next).sort()).toEqual([
      "editor",
      "external",
      "finished",
      "unlistedTerminal",
    ]);
    expect(Object.keys(pruneActivities(next, [task("editor", [])]))).toEqual(["editor"]);
  });
});

describe("indicators", () => {
  it("shows input, working, idle live sessions and ended ones", () => {
    const running = { kind: "running" } as const;
    expect(taskIndicator(activity({ state: "waiting" }), running)).toBe("input");
    expect(taskIndicator(activity({ state: "working" }), running)).toBe("working");
    expect(taskIndicator(activity({ state: "done", unread: true }), undefined)).toBe("idle");
    expect(taskIndicator(undefined, running)).toBe("idle");
    expect(taskIndicator(undefined, { kind: "exited", code: 0 })).toBe("ended");
    expect(taskIndicator(undefined, undefined)).toBeNull();
  });

  it("rolls up to input, then idle, then working", () => {
    expect(strongestIndicator(["working", "idle", null])).toBe("idle");
    expect(strongestIndicator(["idle", "input", "working"])).toBe("input");
    expect(strongestIndicator(["ended", "working"])).toBe("working");
    expect(strongestIndicator([])).toBeNull();
  });
});

describe("chatActivityEvent", () => {
  const chat = (status: ChatInfo["status"], waiting = false): ChatInfo => ({
    taskId: "t1",
    agent: "codex",
    status,
    waiting,
  });

  it("turns chat status changes into working, waiting and done", () => {
    const idle = chat({ kind: "idle" });
    const busy = chat({ kind: "busy" });
    const asking = chat({ kind: "busy" }, true);
    expect(chatActivityEvent(idle, busy, 5)).toMatchObject({ state: "working", at: 5 });
    expect(chatActivityEvent(busy, busy, 6)).toBeNull();
    expect(chatActivityEvent(busy, asking, 7)).toMatchObject({ state: "waiting" });
    expect(chatActivityEvent(asking, asking, 8)).toBeNull();
    expect(chatActivityEvent(asking, busy, 9)).toMatchObject({ state: "working" });
    expect(chatActivityEvent(busy, idle, 10)).toMatchObject({ state: "done", surface: "chat" });
    expect(chatActivityEvent(undefined, idle, 11)).toBeNull();
    expect(chatActivityEvent(busy, chat({ kind: "exited", code: 0 }), 12)).toBe("end");
  });
});

describe("alert settings", () => {
  it("fills defaults, keeps silenced categories and rejects unknown sounds", () => {
    expect(normalizeAgentAlerts(undefined)).toEqual(DEFAULT_AGENT_ALERTS);
    expect(DEFAULT_AGENT_ALERTS.notifications).toEqual({ done: false, waiting: true });
    expect(
      normalizeAgentAlerts({
        sounds: { done: null, waiting: "rm -rf" },
        notifications: { done: true, waiting: "yes" },
      })
    ).toEqual({
      sounds: { done: null, waiting: "Ping" },
      notifications: { done: true, waiting: true },
    });
  });

  it("describes the task and targets the agent's surface", () => {
    expect(
      notificationFor(
        task("t1", [], { linearIssueIdentifier: "WOR-64", linearIssueTitle: "Hooks" }),
        { state: "waiting", agent: "codex", surface: "chat" }
      )
    ).toEqual({
      title: "Codex needs your input",
      body: "WOR-64 · Hooks",
      target: { taskId: "t1", surface: "chat", agent: "codex" },
    });
    expect(
      notificationFor(task("t2", []), { state: "done", agent: "claude", surface: "external" })
    ).toMatchObject({ title: "Claude finished", body: "branch-t2" });
  });

  it("maps notification targets back to embedded surfaces", () => {
    const target = { taskId: "t1", agent: "codex" as const };
    expect(embeddedSurface({ ...target, surface: "chat" })).toEqual({
      kind: "chat",
      agent: "codex",
    });
    expect(embeddedSurface({ ...target, surface: "terminal" })).toEqual({
      kind: "terminal",
      agent: "codex",
    });
    expect(embeddedSurface({ ...target, surface: "editor" })).toEqual({ kind: "editor" });
    expect(embeddedSurface({ ...target, surface: "external" })).toBeNull();
  });
});
