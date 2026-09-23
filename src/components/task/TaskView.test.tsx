// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  term: {
    open: vi.fn(),
    write: vi.fn(),
    reset: vi.fn(),
    focus: vi.fn(),
    dispose: vi.fn(),
    loadAddon: vi.fn(),
    onData: vi.fn(() => ({ dispose: vi.fn() })),
    cols: 80,
    rows: 24,
  },
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
  Channel: class {
    onmessage?: (e: unknown) => void;
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));
vi.mock("@xterm/xterm", () => ({
  Terminal: vi.fn(function () {
    return mocks.term;
  }),
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: vi.fn(function () {
    return { fit: vi.fn() };
  }),
}));
vi.mock("@xterm/xterm/css/xterm.css", () => ({}));

import { TaskView } from "./TaskView";
import { Task } from "../../types";

const task = {
  id: "t1",
  workspaceId: "ws1",
  branchName: "feat/x",
  linearIssueIdentifier: "WOR-87",
  linearIssueTitle: "Embedded agent",
  members: [{ path: "/wt/a" }, { path: "/wt/b" }],
} as unknown as Task;

beforeEach(() => {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as never;
  mocks.invoke.mockReset();
  mocks.term.write.mockClear();
});

afterEach(cleanup);

function renderView(onBack = vi.fn()) {
  render(
    <TaskView
      task={task}
      surface={{ kind: "terminal", agent: "claude" }}
      sidebarCollapsed={false}
      onExpandSidebar={vi.fn()}
      onBack={onBack}
    />
  );
  return onBack;
}

describe("TaskView", () => {
  it("switches agents without closing either session and detaches the previous agent", async () => {
    mocks.invoke.mockResolvedValue({ created: true, status: { kind: "running" }, replay: "" });
    const props = {
      task,
      sidebarCollapsed: false,
      onExpandSidebar: vi.fn(),
      onBack: vi.fn(),
    };
    const view = render(<TaskView {...props} surface={{ kind: "terminal", agent: "claude" }} />);
    await waitFor(() => expect(screen.getByText("running")).toBeInTheDocument());
    view.rerender(<TaskView {...props} surface={{ kind: "terminal", agent: "codex" }} />);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "terminal_open",
        expect.objectContaining({
          taskId: "t1",
          agent: "codex",
          folders: ["/wt/a", "/wt/b"],
        })
      )
    );
    expect(mocks.invoke).toHaveBeenCalledWith("terminal_detach", { taskId: "t1", agent: "claude" });
    expect(mocks.invoke.mock.calls.some(([command]) => command === "terminal_close")).toBe(false);
    view.unmount();
    expect(mocks.invoke).toHaveBeenCalledWith("terminal_detach", { taskId: "t1", agent: "codex" });
  });

  it("switches a terminal to the same agent's chat and back without closing sessions", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open")
        return Promise.resolve({ generation: 1, status: { kind: "idle" }, items: [], pending: [] });
      return Promise.resolve({ created: true, status: { kind: "running" }, replay: "" });
    });
    renderView();
    await waitFor(() => expect(screen.getByText("running")).toBeInTheDocument());
    const view = screen.getByRole("group", { name: "View" });
    fireEvent.click(within(view).getByRole("button", { name: "Chat" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "chat_open",
        expect.objectContaining({ taskId: "t1", agent: "claude", folders: ["/wt/a", "/wt/b"] })
      )
    );
    expect(mocks.invoke).toHaveBeenCalledWith("terminal_detach", { taskId: "t1", agent: "claude" });
    expect(screen.queryByRole("group", { name: "Agent" })).not.toBeInTheDocument();

    fireEvent.click(
      within(screen.getByRole("group", { name: "View" })).getByRole("button", { name: "Terminal" })
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_detach", {
        taskId: "t1",
        agent: "claude",
        generation: 1,
      })
    );
    expect(
      mocks.invoke.mock.calls.some(
        ([command]) => command === "chat_close" || command === "terminal_close"
      )
    ).toBe(false);
  });

  it("closes an embedded editor before returning to the task list", async () => {
    let finishClose: (() => void) | undefined;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "vscode_probe") return Promise.resolve({ ready: false });
      if (command === "vscode_close")
        return new Promise<void>((resolve) => {
          finishClose = resolve;
        });
      return Promise.resolve();
    });
    const onBack = vi.fn();
    render(
      <TaskView
        task={task}
        surface={{ kind: "editor" }}
        sidebarCollapsed={false}
        onExpandSidebar={vi.fn()}
        onBack={onBack}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Close editor" }));
    expect(mocks.invoke).toHaveBeenCalledWith("vscode_close", { taskId: task.id });
    expect(onBack).not.toHaveBeenCalled();
    finishClose?.();
    await waitFor(() => expect(onBack).toHaveBeenCalledOnce());
  });
  it("shows the task header and attaches to the task terminal", async () => {
    mocks.invoke.mockResolvedValue({ created: true, status: { kind: "running" }, replay: "old" });
    renderView();
    expect(screen.getByText("WOR-87")).toBeInTheDocument();
    expect(screen.getByText("Embedded agent")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("running")).toBeInTheDocument());
    const [command, args] = mocks.invoke.mock.calls[0];
    expect(command).toBe("terminal_open");
    expect(args).toMatchObject({
      taskId: "t1",
      agent: "claude",
      folders: ["/wt/a", "/wt/b"],
      branchName: "feat/x",
    });
    expect(mocks.term.write).toHaveBeenCalledWith("old");
  });

  it("goes back from the button and from meta+[", async () => {
    mocks.invoke.mockResolvedValue({ created: true, status: { kind: "running" }, replay: "" });
    const onBack = renderView();
    fireEvent.click(screen.getByLabelText("Back to tasks (⌘[)"));
    fireEvent.keyDown(document.body, { key: "[", metaKey: true });
    expect(onBack).toHaveBeenCalledTimes(2);
  });

  it("offers a restart when the session has ended", async () => {
    mocks.invoke.mockResolvedValue({
      created: false,
      status: { kind: "exited", code: 1 },
      replay: "",
    });
    renderView();
    const restart = await screen.findByRole("button", { name: /Restart session/ });
    fireEvent.click(restart);
    await waitFor(() =>
      expect(mocks.invoke.mock.calls.filter(([c]) => c === "terminal_open")).toHaveLength(2)
    );
    expect(mocks.term.reset).toHaveBeenCalled();
  });
});
