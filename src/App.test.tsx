// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import App from "./App";
import { ZoomControls } from "./components/ui/ZoomControls";
import { DEFAULT_STATE } from "./types";

const mocks = vi.hoisted(() => ({
  selectWorkspace: vi.fn(),
  historyBack: vi.fn(),
  historyForward: vi.fn(),
  historyParams: { onNavigate: (() => undefined) as (entry: unknown) => void },
  useStore: vi.fn(),
  useDoctor: vi.fn(),
  listen: vi.fn(),
  unlisten: vi.fn(),
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
  invoke: mocks.invoke,
}));

vi.mock("./hooks/useStore", () => ({ useStore: mocks.useStore }));
vi.mock("./hooks/useUpdater", () => ({ useUpdater: vi.fn() }));
vi.mock("./hooks/useNavigationHistory", () => ({
  useNavigationHistory: (params: { onNavigate: (entry: unknown) => void }) => ({
    ...(mocks.historyParams = params),
    entries: [],
    canGoBack: true,
    canGoForward: true,
    back: mocks.historyBack,
    forward: mocks.historyForward,
    recordWorkspaceVisit: vi.fn(),
    recordTaskVisit: vi.fn(),
  }),
}));
vi.mock("./hooks/useWindowDrag", () => ({ useWindowDrag: vi.fn() }));
vi.mock("./hooks/useLinearOrgKeyBackfill", () => ({ useLinearOrgKeyBackfill: vi.fn() }));
// Shells out and calls Linear; keep it out of a render test.
vi.mock("./hooks/useDoctor", () => ({
  useDoctor: mocks.useDoctor,
}));
vi.mock("./components/setup/SetupWizard", () => ({ SetupWizard: () => null }));
vi.mock("./components/sidebar/WorkspaceList", () => ({
  WorkspaceList: ({ onOpenDoctor }: { onOpenDoctor: () => void }) => (
    <button onClick={onOpenDoctor}>Dependencies</button>
  ),
}));
vi.mock("./components/worktree/WorktreeList", () => ({ WorktreeList: () => null }));
vi.mock("./components/search/QuickSearchModal", () => ({
  QuickSearchModal: ({ open }: { open: boolean }) =>
    open ? <div role="dialog">Task search</div> : null,
}));

const workspaces = [
  { id: "first", name: "First", repos: [] },
  { id: "second", name: "Second", repos: [] },
];

beforeEach(() => {
  localStorage.clear();
  mocks.selectWorkspace.mockClear();
  mocks.historyBack.mockClear();
  mocks.historyForward.mockClear();
  mocks.listen.mockReset().mockResolvedValue(mocks.unlisten);
  mocks.unlisten.mockReset();
  mocks.invoke.mockReset().mockResolvedValue(undefined);
  mocks.useDoctor.mockReturnValue({ report: null, running: false, recheck: vi.fn() });
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

describe("native editor navigation", () => {
  it("opens task search from the native editor event", async () => {
    const view = render(<App />);
    expect(mocks.listen).toHaveBeenCalledWith("editor-navigate", expect.any(Function));
    expect(view.queryByRole("dialog")).toBeNull();
    const notify = mocks.listen.mock.calls[0][1];
    await act(async () => notify({ payload: "search" }));
    expect(view.getByRole("dialog").textContent).toBe("Task search");
  });

  it("releases a native subscription that finishes registering after unmount", async () => {
    let registered!: (stop: () => void) => void;
    mocks.listen.mockReturnValueOnce(
      new Promise<() => void>((resolve) => {
        registered = resolve;
      })
    );
    const view = render(<App />);
    view.unmount();
    expect(mocks.unlisten).not.toHaveBeenCalled();
    await act(async () => registered(mocks.unlisten));
    expect(mocks.unlisten).toHaveBeenCalledOnce();
  });
});

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

describe("doctor alerts", () => {
  const git = {
    id: "cli:git",
    label: "git",
    scope: "app",
    status: "missing",
    severity: "error",
    reason: "Creates worktrees.",
    detail: null,
  };
  const pnpm = {
    id: "cli:pnpm",
    label: "pnpm",
    scope: "repository",
    status: "missing",
    severity: "warning",
    reason: "Installs repository dependencies.",
    detail: null,
  };

  it("keeps repo suggestions in the modal without showing a startup alert", () => {
    mocks.useDoctor.mockReturnValue({
      report: { checks: [pnpm], errors: 0, warnings: 0 },
      running: false,
      recheck: vi.fn(),
    });
    const view = render(<App />);
    expect(view.queryByRole("status")).toBeNull();
    expect(view.queryByText("pnpm")).toBeNull();
    fireEvent.click(view.getByRole("button", { name: "Dependencies" }));
    expect(view.getByText("pnpm")).toBeTruthy();
    expect(view.queryByText("Optional for repos")).toBeNull();
    expect(
      view.getByText("Optional for repositories. Installs repository dependencies.")
    ).toBeTruthy();
    expect(view.getByText("1 optional repo suggestion. Nothing here blocks the app.")).toBeTruthy();
  });

  it("retries Keychain access before validating restored Linear keys", async () => {
    const recheck = vi.fn();
    const store = mocks.useStore();
    const restored = {
      ...store.state,
      workspaces: [{ ...workspaces[0], linearApiKey: "lin_recovered" }],
    };
    const retryKeychain = vi.fn().mockResolvedValue(restored);
    mocks.useStore.mockReturnValue({ ...store, keychainError: "Access denied", retryKeychain });
    mocks.useDoctor.mockReturnValue({
      report: {
        checks: [{ ...git, id: "keychain-access", label: "Keychain access" }],
        errors: 1,
        warnings: 0,
      },
      running: false,
      recheck,
    });
    const view = render(<App />);
    fireEvent.click(view.getByRole("button", { name: "Review" }));
    fireEvent.click(view.getByRole("button", { name: "Re-check" }));
    await waitFor(() =>
      expect(recheck).toHaveBeenCalledWith(
        expect.objectContaining({
          keychainError: null,
          linearKeys: [{ label: "First", key: "lin_recovered" }],
        })
      )
    );
    expect(retryKeychain).toHaveBeenCalledOnce();
  });

  it("keeps the Keychain failure visible when retry is denied", async () => {
    const recheck = vi.fn();
    const store = mocks.useStore();
    mocks.useStore.mockReturnValue({
      ...store,
      keychainError: "Access denied",
      retryKeychain: vi.fn().mockRejectedValue(new Error("Still denied")),
    });
    mocks.useDoctor.mockReturnValue({
      report: {
        checks: [{ ...git, id: "keychain-access", label: "Keychain access" }],
        errors: 1,
        warnings: 0,
      },
      running: false,
      recheck,
    });
    const view = render(<App />);
    fireEvent.click(view.getByRole("button", { name: "Review" }));
    fireEvent.click(view.getByRole("button", { name: "Re-check" }));
    await waitFor(() =>
      expect(recheck).toHaveBeenCalledWith(
        expect.objectContaining({
          keychainError: "Error: Still denied",
        })
      )
    );
  });

  it("only names app dependencies in the alert and allows reviewing and dismissing it", () => {
    mocks.useDoctor.mockReturnValue({
      report: { checks: [git, pnpm], errors: 1, warnings: 0 },
      running: false,
      recheck: vi.fn(),
    });
    const view = render(<App />);
    expect(view.getByRole("status").textContent).toContain("WorktreeManager needs attention: git");
    expect(view.getByRole("status").textContent).not.toContain("pnpm");
    fireEvent.click(view.getByRole("button", { name: "Review" }));
    expect(view.getByText("Creates worktrees.")).toBeTruthy();
    fireEvent.keyDown(document, { key: "Escape" });
    fireEvent.click(view.getByRole("button", { name: "Dismiss" }));
    expect(view.queryByRole("status")).toBeNull();
    expect(view.getByRole("button", { name: "Dependencies" })).toBeTruthy();
  });
});

describe("navigation history shortcuts", () => {
  it.each([
    ["ArrowLeft", "historyBack"],
    ["ArrowRight", "historyForward"],
  ] as const)("maps Cmd+%s to %s", (key, handler) => {
    render(<App />);
    fireEvent.keyDown(window, { key, metaKey: true });
    expect(mocks[handler]).toHaveBeenCalledOnce();
  });

  it("switches workspace when history navigation lands on another one", () => {
    render(<App />);
    act(() => mocks.historyParams.onNavigate({ kind: "workspace", workspaceId: "first", at: "" }));
    expect(mocks.selectWorkspace).toHaveBeenCalledExactlyOnceWith("first");
  });

  it("leaves Cmd+Arrow alone while typing in a text field", () => {
    render(<App />);
    const input = document.createElement("input");
    document.body.appendChild(input);
    fireEvent.keyDown(input, { key: "ArrowLeft", metaKey: true });
    expect(mocks.historyBack).not.toHaveBeenCalled();
  });
});
