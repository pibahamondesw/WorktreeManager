// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { useStore } from "./useStore";
import { presetFor } from "../themes";
import { DEFAULT_STATE } from "../types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock("../services/store", () => ({
  loadState: vi.fn(),
  restoreKeychainSecrets: vi.fn(),
  loadEditorApp: vi.fn().mockResolvedValue("cursor"),
  loadThemeId: vi.fn().mockResolvedValue("default"),
  loadCustomColors: vi.fn().mockResolvedValue(null),
  loadSidebarCollapsed: vi.fn().mockResolvedValue(false),
  loadAgentViews: vi.fn().mockResolvedValue({ claude: "chat", codex: "chat" }),
  persist: vi.fn().mockResolvedValue(undefined),
}));

import { loadState, persist } from "../services/store";

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
  it("starts with VS Code embedded and restores an existing editor preference", async () => {
    const { result } = renderHook(() => useStore());

    expect(result.current.editorApp).toBe("vscode-web");
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.editorApp).toBe("cursor");
  });

  it("publishes CLI changes to React only after persistence and restores them on load", async () => {
    const { result, unmount } = renderHook(() => useStore());
    await waitFor(() => expect(result.current.loading).toBe(false));
    let saved!: () => void;
    vi.mocked(persist).mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          saved = resolve;
        })
    );
    let writing!: Promise<void>;
    act(() => {
      writing = result.current.operations.change(() => ({
        tasks: [
          {
            id: "cli-task",
            workspaceId: "a",
            branchName: "feature",
            members: [],
            createdAt: "2026-09-11",
          },
        ],
      }));
    });
    await waitFor(() => expect(saved).toBeTypeOf("function"));
    expect(result.current.state.tasks).toEqual([]);
    await act(async () => {
      saved();
      await writing;
    });
    expect(result.current.selectedTasks[0].id).toBe("cli-task");
    vi.mocked(loadState).mockResolvedValue(result.current.state);
    unmount();
    const reloaded = renderHook(() => useStore());
    await waitFor(() => expect(reloaded.result.current.loading).toBe(false));
    expect(reloaded.result.current.selectedTasks[0].id).toBe("cli-task");
  });

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

describe("useStore preferences", () => {
  it("restores the sidebar and shows a banner when saving fails", async () => {
    const { result } = renderHook(() => useStore());
    await waitFor(() => expect(result.current.loading).toBe(false));
    vi.mocked(persist).mockRejectedValueOnce(new Error("disk full"));

    await act(async () => result.current.toggleSidebarCollapsed());

    expect(result.current.sidebarCollapsed).toBe(false);
    expect(result.current.persistError).toBe("Failed to save sidebar preference");
    act(() => result.current.dismissPersistError());
    expect(result.current.persistError).toBeNull();
  });

  it("saves the first custom theme and its seeded colors in one batch", async () => {
    const { result } = renderHook(() => useStore());
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => result.current.updateThemeId("custom"));

    const colors = presetFor("default").colors;
    expect(persist).toHaveBeenLastCalledWith([
      ["themeId", "custom"],
      ["customTheme", colors],
    ]);
    expect(result.current.themeId).toBe("custom");
    expect(result.current.customColors).toEqual(colors);
    expect(document.documentElement.style.getPropertyValue("--color-accent")).toBe(colors.accent);
  });

  it("reflects consecutive color edits immediately while the first save is pending", async () => {
    const { result } = renderHook(() => useStore());
    await waitFor(() => expect(result.current.loading).toBe(false));
    await act(async () => result.current.updateThemeId("custom"));
    let finishSave!: () => void;
    vi.mocked(persist).mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishSave = resolve;
        })
    );
    const writes: Promise<void>[] = [];

    act(() => {
      writes.push(
        result.current.updateCustomColors({ ...result.current.customColors, accent: "#112233" })
      );
    });
    await waitFor(() => expect(finishSave).toBeTypeOf("function"));
    act(() => {
      writes.push(
        result.current.updateCustomColors({
          ...result.current.customColors,
          "bg-primary": "#223344",
        })
      );
    });
    expect(result.current.customColors?.["bg-primary"]).toBe("#223344");
    act(() => {
      writes.push(
        result.current.updateCustomColors({ ...result.current.customColors, success: "#334455" })
      );
    });
    expect(result.current.customColors?.success).toBe("#334455");

    await act(async () => {
      finishSave();
      await Promise.all(writes);
    });

    expect(result.current.customColors).toMatchObject({
      accent: "#112233",
      "bg-primary": "#223344",
      success: "#334455",
    });
    expect(persist).toHaveBeenLastCalledWith([["customTheme", result.current.customColors]]);
  });

  it("restores the rendered theme and colors when the batch fails", async () => {
    const { result } = renderHook(() => useStore());
    await waitFor(() => expect(result.current.loading).toBe(false));
    await act(async () => result.current.updateThemeId("ocean"));
    vi.mocked(persist).mockRejectedValueOnce(new Error("disk full"));

    await act(async () => result.current.updateThemeId("custom"));

    expect(result.current.themeId).toBe("ocean");
    expect(result.current.customColors).toBeNull();
    expect(document.documentElement.style.getPropertyValue("--color-accent")).toBe(
      presetFor("ocean").colors.accent
    );
    expect(result.current.persistError).toBe("Failed to save theme preference");
  });
});
