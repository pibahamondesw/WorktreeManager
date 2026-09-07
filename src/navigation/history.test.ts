import { describe, expect, it } from "vitest";
import {
  EMPTY_HISTORY,
  MAX_HISTORY_ENTRIES,
  NavigationEntry,
  appendVisit,
  historyFromEntries,
  seedSession,
  isNavigationEntry,
  lastTaskVisits,
  nextNavigableIndex,
  pushEntry,
} from "./history";

const ws = (id: string, at = "2026-09-06T00:00:00.000Z"): NavigationEntry => ({
  kind: "workspace",
  workspaceId: id,
  at,
});
const task = (
  id: string,
  workspaceId = "w1",
  at = "2026-09-06T00:00:00.000Z"
): NavigationEntry => ({
  kind: "task",
  taskId: id,
  workspaceId,
  at,
});

describe("lastTaskVisits", () => {
  it("maps each task to its latest visit and ignores workspace entries", () => {
    const visits = lastTaskVisits([
      task("t1", "w1", "2026-09-01T00:00:00.000Z"),
      ws("w1"),
      task("t2", "w1", "2026-09-02T00:00:00.000Z"),
      task("t1", "w1", "2026-09-03T00:00:00.000Z"),
    ]);
    expect(visits.get("t1")).toBe("2026-09-03T00:00:00.000Z");
    expect(visits.get("t2")).toBe("2026-09-02T00:00:00.000Z");
    expect(visits.size).toBe(2);
  });
});

describe("pushEntry", () => {
  it("appends and moves the cursor to the new entry", () => {
    const h = pushEntry(pushEntry(EMPTY_HISTORY, ws("a")), ws("b"));
    expect(h.entries.map((e) => e.workspaceId)).toEqual(["a", "b"]);
    expect(h.cursor).toBe(1);
  });

  it("refreshes the timestamp instead of duplicating the current target", () => {
    const h = pushEntry(pushEntry(EMPTY_HISTORY, ws("a", "t1")), ws("a", "t2"));
    expect(h.entries).toEqual([ws("a", "t2")]);
    expect(h.cursor).toBe(0);
  });

  it("treats a task and its workspace as different targets", () => {
    const h = pushEntry(pushEntry(EMPTY_HISTORY, ws("w1")), task("t1"));
    expect(h.entries).toHaveLength(2);
  });

  it("drops entries ahead of the cursor when pushing after going back", () => {
    const h = { ...pushEntry(pushEntry(EMPTY_HISTORY, ws("a")), ws("b")), cursor: 0 };
    const next = pushEntry(h, ws("c"));
    expect(next.entries.map((e) => e.workspaceId)).toEqual(["a", "c"]);
    expect(next.cursor).toBe(1);
  });

  it("drops the oldest entries past the cap", () => {
    let h = EMPTY_HISTORY;
    for (let i = 0; i <= MAX_HISTORY_ENTRIES; i++) h = pushEntry(h, ws(`w${i}`));
    expect(h.entries).toHaveLength(MAX_HISTORY_ENTRIES);
    expect(h.entries[0].workspaceId).toBe("w1");
    expect(h.cursor).toBe(MAX_HISTORY_ENTRIES - 1);
  });
});

describe("nextNavigableIndex", () => {
  const h = historyFromEntries([ws("a"), task("t1"), ws("b"), ws("c")]);

  it("steps back and forward from the cursor", () => {
    expect(nextNavigableIndex(h, -1, () => true)).toBe(2);
    expect(nextNavigableIndex({ ...h, cursor: 1 }, 1, () => true)).toBe(2);
  });

  it("skips entries that no longer resolve", () => {
    const skipTasks = (e: NavigationEntry) => e.kind !== "task";
    expect(nextNavigableIndex({ ...h, cursor: 2 }, -1, skipTasks)).toBe(0);
  });

  it("returns null at either end", () => {
    expect(nextNavigableIndex(h, 1, () => true)).toBeNull();
    expect(nextNavigableIndex({ ...h, cursor: 0 }, -1, () => true)).toBeNull();
    expect(nextNavigableIndex(h, -1, () => false)).toBeNull();
  });
});

describe("historyFromEntries", () => {
  it("starts at the most recent entry", () => {
    expect(historyFromEntries([ws("a"), ws("b")]).cursor).toBe(1);
    expect(historyFromEntries([]).cursor).toBe(-1);
  });
});

describe("isNavigationEntry", () => {
  it("accepts both kinds and rejects malformed values", () => {
    expect(isNavigationEntry(ws("a"))).toBe(true);
    expect(isNavigationEntry(task("t1"))).toBe(true);
    expect(isNavigationEntry({ kind: "task", workspaceId: "w", at: "x" })).toBe(false);
    expect(isNavigationEntry({ kind: "other", workspaceId: "w", at: "x" })).toBe(false);
    expect(isNavigationEntry(null)).toBe(false);
  });
});

describe("appendVisit", () => {
  it("keeps every visit, refreshing only a repeated last target", () => {
    const log = appendVisit(appendVisit(appendVisit([], ws("a", "t1")), ws("b")), ws("b", "t2"));
    expect(log).toEqual([ws("a", "t1"), ws("b", "t2")]);
  });
});

describe("seedSession", () => {
  it("starts at the previous session's last visit and replays what was recorded meanwhile", () => {
    const h = seedSession([ws("a"), ws("b")], [ws("c")]);
    expect(h.entries.map((e) => e.workspaceId)).toEqual(["b", "c"]);
    expect(h.cursor).toBe(1);
  });

  it("does not duplicate the seed when the session reopened on the same target", () => {
    const h = seedSession([ws("a"), ws("b", "t1")], [ws("b", "t2")]);
    expect(h.entries).toEqual([ws("b", "t2")]);
  });

  it("is empty for a first run", () => {
    expect(seedSession([], [])).toEqual(EMPTY_HISTORY);
  });
});
