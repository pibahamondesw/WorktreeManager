// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import App from "./App";
import { ZoomControls } from "./components/ui/ZoomControls";
import { DEFAULT_STATE } from "./types";

const mocks = vi.hoisted(() => ({
  selectWorkspace: vi.fn(),
  useStore: vi.fn(),
}));

vi.mock("./hooks/useStore", () => ({ useStore: mocks.useStore }));
vi.mock("./hooks/useUpdater", () => ({ useUpdater: vi.fn() }));
vi.mock("./hooks/useWindowDrag", () => ({ useWindowDrag: vi.fn() }));
vi.mock("./hooks/useLinearOrgKeyBackfill", () => ({ useLinearOrgKeyBackfill: vi.fn() }));
vi.mock("./components/setup/SetupWizard", () => ({ SetupWizard: () => null }));
vi.mock("./components/sidebar/WorkspaceList", () => ({ WorkspaceList: () => null }));
vi.mock("./components/worktree/WorktreeList", () => ({ WorktreeList: () => null }));
vi.mock("./components/search/QuickSearchModal", () => ({ QuickSearchModal: () => null }));

const workspaces = [
  { id: "first", name: "First", repos: [] },
  { id: "second", name: "Second", repos: [] },
];

beforeEach(() => {
  localStorage.clear();
  mocks.selectWorkspace.mockClear();
  mocks.useStore.mockReturnValue({
    state: {
      ...DEFAULT_STATE,
      setup: { ...DEFAULT_STATE.setup, isComplete: true },
      workspaces,
      selectedWorkspaceId: "second",
    },
    selectedWorkspace: workspaces[1],
    selectedTasks: [],
    selectWorkspace: mocks.selectWorkspace,
  });
});
afterEach(cleanup);

describe("workspace keyboard navigation with zoom mounted", () => {
  it("maps Cmd+0 to the first workspace without changing zoom", () => {
    render(
      <>
        <App />
        <ZoomControls />
      </>
    );
    fireEvent.keyDown(window, { key: "+", code: "Equal", metaKey: true });
    expect(document.documentElement.style.fontSize).toBe("17.6px");
    fireEvent.keyDown(window, { key: "0", code: "Digit0", metaKey: true });
    expect(mocks.selectWorkspace).toHaveBeenCalledExactlyOnceWith("first");
    expect(document.documentElement.style.fontSize).toBe("17.6px");
  });

  it("uses the current sidebar order after workspaces are reordered", () => {
    const { rerender } = render(
      <>
        <App />
        <ZoomControls />
      </>
    );
    const store = mocks.useStore();
    mocks.useStore.mockReturnValue({
      ...store,
      state: {
        ...store.state,
        workspaces: [...workspaces].reverse(),
        selectedWorkspaceId: "first",
      },
    });
    rerender(
      <>
        <App />
        <ZoomControls />
      </>
    );
    fireEvent.keyDown(window, { key: "0", code: "Digit0", metaKey: true });
    expect(mocks.selectWorkspace).toHaveBeenCalledExactlyOnceWith("second");
  });

  it("ignores indices with no workspace", () => {
    render(
      <>
        <App />
        <ZoomControls />
      </>
    );
    fireEvent.keyDown(window, { key: "9", code: "Digit9", metaKey: true });
    expect(mocks.selectWorkspace).not.toHaveBeenCalled();
  });
});
