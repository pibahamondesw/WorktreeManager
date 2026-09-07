// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { useNavigationHistory } from "./useNavigationHistory";
import { NavigationEntry } from "../navigation/history";
import { Task } from "../types";

const mocks = vi.hoisted(() => ({
  loadNavigationHistory: vi.fn(),
  persist: vi.fn(),
}));

vi.mock("../services/store", () => ({
  loadNavigationHistory: mocks.loadNavigationHistory,
  persist: mocks.persist,
}));

const task = (id: string, workspaceId: string): Task => ({
  id,
  workspaceId,
  branchName: id,
  members: [],
  createdAt: "2026-09-06T00:00:00.000Z",
});

beforeEach(() => {
  mocks.loadNavigationHistory.mockReset().mockResolvedValue([]);
  mocks.persist.mockReset().mockResolvedValue(undefined);
});
afterEach(cleanup);

function setup(isNavigable: (e: NavigationEntry) => boolean = () => true) {
  const onNavigate = vi.fn();
  const hook = renderHook(() => useNavigationHistory({ isNavigable, onNavigate }));
  return { ...hook, onNavigate };
}

describe("useNavigationHistory", () => {
  it("records visits, persists them and walks back and forward without re-recording", async () => {
    const { result, onNavigate } = setup();
    await waitFor(() => expect(mocks.loadNavigationHistory).toHaveBeenCalled());

    act(() => result.current.recordWorkspaceVisit("w1"));
    act(() => result.current.recordTaskVisit(task("t1", "w1")));
    act(() => result.current.recordWorkspaceVisit("w2"));

    expect(result.current.entries.map((e) => e.kind)).toEqual(["workspace", "task", "workspace"]);
    expect(mocks.persist).toHaveBeenLastCalledWith([["navigationHistory", result.current.entries]]);
    expect(result.current.canGoBack).toBe(true);
    expect(result.current.canGoForward).toBe(false);

    act(() => result.current.back());
    expect(onNavigate).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: "task", taskId: "t1" })
    );
    expect(result.current.canGoForward).toBe(true);

    act(() => result.current.back());
    expect(onNavigate).toHaveBeenLastCalledWith(expect.objectContaining({ workspaceId: "w1" }));
    expect(result.current.canGoBack).toBe(false);

    act(() => result.current.forward());
    expect(onNavigate).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: "task", taskId: "t1" })
    );
    expect(result.current.entries).toHaveLength(3);
    expect(mocks.persist).toHaveBeenCalledTimes(3);
  });

  it("truncates the session stack when visiting after going back but keeps the persisted log", async () => {
    const { result } = setup();
    await waitFor(() => expect(mocks.loadNavigationHistory).toHaveBeenCalled());

    act(() => result.current.recordWorkspaceVisit("w1"));
    act(() => result.current.recordWorkspaceVisit("w2"));
    act(() => result.current.back());
    act(() => result.current.recordWorkspaceVisit("w3"));

    expect(result.current.entries.map((e) => e.workspaceId)).toEqual(["w1", "w3"]);
    expect(result.current.canGoForward).toBe(false);
    const persisted = mocks.persist.mock.lastCall?.[0][0][1] as NavigationEntry[];
    expect(persisted.map((e) => e.workspaceId)).toEqual(["w1", "w2", "w3"]);
  });

  it("seeds the session with the previous session's last visit only", async () => {
    mocks.loadNavigationHistory.mockResolvedValue([
      { kind: "workspace", workspaceId: "w1", at: "2026-09-05T00:00:00.000Z" },
      { kind: "workspace", workspaceId: "w2", at: "2026-09-06T00:00:00.000Z" },
    ]);
    const { result, onNavigate } = setup();
    await waitFor(() => expect(result.current.entries).toHaveLength(1));
    expect(result.current.canGoBack).toBe(false);

    act(() => result.current.recordWorkspaceVisit("w3"));
    act(() => result.current.back());
    expect(onNavigate).toHaveBeenCalledWith(expect.objectContaining({ workspaceId: "w2" }));
    expect(result.current.canGoBack).toBe(false);

    const persisted = mocks.persist.mock.lastCall?.[0][0][1] as NavigationEntry[];
    expect(persisted.map((e) => e.workspaceId)).toEqual(["w1", "w2", "w3"]);
  });

  it("merges a visit recorded before the store loaded without duplicating the seed", async () => {
    let resolveLoad: (v: NavigationEntry[]) => void = () => undefined;
    mocks.loadNavigationHistory.mockReturnValue(new Promise((r) => (resolveLoad = r)));
    const { result } = setup();

    act(() => result.current.recordWorkspaceVisit("w2"));
    await act(async () => {
      resolveLoad([{ kind: "workspace", workspaceId: "w2", at: "2026-09-05T00:00:00.000Z" }]);
    });

    expect(result.current.entries).toHaveLength(1);
    expect(result.current.canGoBack).toBe(false);
  });

  it("skips entries that no longer resolve", async () => {
    const { result, onNavigate } = setup((e) => e.kind !== "task");
    await waitFor(() => expect(mocks.loadNavigationHistory).toHaveBeenCalled());

    act(() => result.current.recordWorkspaceVisit("w1"));
    act(() => result.current.recordTaskVisit(task("gone", "w1")));
    act(() => result.current.recordWorkspaceVisit("w2"));

    act(() => result.current.back());
    expect(onNavigate).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ workspaceId: "w1" })
    );
  });

  it("does nothing when there is nowhere to go", async () => {
    const { result, onNavigate } = setup();
    await waitFor(() => expect(mocks.loadNavigationHistory).toHaveBeenCalled());
    act(() => result.current.back());
    act(() => result.current.forward());
    expect(onNavigate).not.toHaveBeenCalled();
  });
});
