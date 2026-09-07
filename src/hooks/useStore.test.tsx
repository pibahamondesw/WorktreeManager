// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { useStore } from "./useStore";
import { DEFAULT_STATE } from "../types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock("../services/store", () => ({
  loadState: vi.fn(),
  restoreKeychainSecrets: vi.fn(),
  loadEditorApp: vi.fn().mockResolvedValue("cursor"),
  loadThemeId: vi.fn().mockResolvedValue("default"),
  loadCustomColors: vi.fn().mockResolvedValue(null),
  loadSidebarCollapsed: vi.fn().mockResolvedValue(false),
  persist: vi.fn().mockResolvedValue(undefined),
}));

import { loadState } from "../services/store";

const workspaces = [
  { id: "a", name: "A", repos: [] },
  { id: "b", name: "B", repos: [] },
];

beforeEach(() => {
  vi.mocked(loadState).mockResolvedValue({
    ...DEFAULT_STATE,
    setup: { ...DEFAULT_STATE.setup, isComplete: true },
    workspaces,
    selectedWorkspaceId: "a",
  });
});
afterEach(cleanup);

describe("useStore workspace switching", () => {
  it("stops showing skeletons when returning to a loaded workspace before the other one finished", async () => {
    const { result } = renderHook(() => useStore());
    await waitFor(() => expect(result.current.loading).toBe(false));

    act(() => result.current.clearWorkspaceSwitching("a"));
    expect(result.current.workspaceSwitching).toBe(false);

    act(() => result.current.selectWorkspace("b"));
    expect(result.current.workspaceSwitching).toBe(true);

    act(() => result.current.selectWorkspace("a"));
    expect(result.current.workspaceSwitching).toBe(false);
  });
});
