// @vitest-environment jsdom
import { expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useTaskActivityRecorder } from "./useTaskActivityRecorder";
import { AgentActivities, AgentActivity } from "../services/agentActivity";
import { Task } from "../types";

const HOUR = 60 * 60 * 1000;
const SAVED = Date.parse("2026-03-01T00:00:00.000Z");
const task = (id: string): Task => ({
  id,
  workspaceId: "w1",
  branchName: id,
  members: [],
  createdAt: "2026-01-01T00:00:00.000Z",
  lastActiveAt: new Date(SAVED).toISOString(),
});
const iso = (ms: number) => new Date(ms).toISOString();

function renderRecorder(record = vi.fn().mockResolvedValue(undefined)) {
  const props = {
    tasks: [task("visited"), task("agent"), task("recent")],
    lastVisitAt: new Map([
      ["visited", iso(SAVED + 2 * HOUR)],
      ["recent", iso(SAVED + HOUR / 2)],
    ]),
    agentActivities: { agent: { at: SAVED + 3 * HOUR } as AgentActivity } as AgentActivities,
    record,
  };
  const view = renderHook((current: typeof props) => useTaskActivityRecorder(current), {
    initialProps: props,
  });
  return {
    record,
    rerender: (next: Partial<typeof props>) => view.rerender({ ...props, ...next }),
  };
}

it("saves visits and agent events at least an hour newer than the saved activity", () => {
  const { record } = renderRecorder();
  expect(record).toHaveBeenCalledWith({
    visited: iso(SAVED + 2 * HOUR),
    agent: iso(SAVED + 3 * HOUR),
  });
});

it("waits for a save to finish before saving again", async () => {
  let finish!: () => void;
  const record = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
  const { rerender } = renderRecorder(record);
  rerender({ agentActivities: {} });
  expect(record).toHaveBeenCalledOnce();
  await act(async () => finish());
  rerender({ agentActivities: {} });
  expect(record).toHaveBeenCalledTimes(2);
});

it("retries after a failed save", async () => {
  const record = vi.fn().mockRejectedValueOnce(new Error("disk full")).mockResolvedValue(undefined);
  const { rerender } = renderRecorder(record);
  await act(async () => undefined);
  rerender({ agentActivities: {} });
  expect(record).toHaveBeenCalledTimes(2);
});

it("saves nothing without activity newer than an hour after the saved one", () => {
  const record = vi.fn().mockResolvedValue(undefined);
  renderHook(() =>
    useTaskActivityRecorder({
      tasks: [task("recent")],
      lastVisitAt: new Map([["recent", iso(SAVED + HOUR / 2)]]),
      agentActivities: {},
      record,
    })
  );
  expect(record).not.toHaveBeenCalled();
});
