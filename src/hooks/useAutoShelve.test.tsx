// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useAutoShelve } from "./useAutoShelve";
import { Task } from "../types";

const NOW = Date.parse("2026-03-01T00:00:00.000Z");
const task = (id: string, issue?: string): Task => ({
  id,
  workspaceId: "w1",
  branchName: id,
  linearIssueId: issue,
  members: [],
  createdAt: "2026-01-01T00:00:00.000Z",
});
const DONE = { status: { name: "Done", type: "completed" }, prs: [] };

afterEach(() => vi.restoreAllMocks());

function renderAutoShelve(tasks: Task[], shelveTasks = vi.fn().mockResolvedValue(undefined)) {
  const props = {
    tasks,
    linearInfo: { done: DONE },
    gitStatuses: {},
    settings: { inactiveDays: 30, since: "2026-01-01T00:00:00.000Z" },
    now: NOW,
    shelveTasks,
  };
  const view = renderHook((current: typeof props) => useAutoShelve(current), {
    initialProps: props,
  });
  return {
    shelveTasks,
    rerender: (next: Partial<typeof props>) => view.rerender({ ...props, ...next }),
  };
}

it("moves the tasks whose work is over in one call", async () => {
  const { shelveTasks } = renderAutoShelve([
    task("a", "done"),
    task("b", "open"),
    task("c", "done"),
  ]);
  await waitFor(() => expect(shelveTasks).toHaveBeenCalledWith(["a", "c"]));
  expect(shelveTasks).toHaveBeenCalledOnce();
});

it("does not ask again for tasks whose move is still being saved", async () => {
  let finish!: () => void;
  const shelveTasks = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
  const { rerender } = renderAutoShelve([task("a", "done")], shelveTasks);
  rerender({ now: NOW + 1 });
  expect(shelveTasks).toHaveBeenCalledOnce();
  await act(async () => finish());
  rerender({ now: NOW + 2 });
  expect(shelveTasks).toHaveBeenCalledTimes(2);
});

it("asks again after a failed save", async () => {
  const shelveTasks = vi
    .fn()
    .mockRejectedValueOnce(new Error("disk full"))
    .mockResolvedValue(undefined);
  const { rerender } = renderAutoShelve([task("a", "done")], shelveTasks);
  await act(async () => undefined);
  rerender({ now: NOW + 1 });
  expect(shelveTasks).toHaveBeenCalledTimes(2);
});

it("does nothing while no task qualifies", () => {
  const { shelveTasks } = renderAutoShelve([task("b", "open")]);
  expect(shelveTasks).not.toHaveBeenCalled();
});
