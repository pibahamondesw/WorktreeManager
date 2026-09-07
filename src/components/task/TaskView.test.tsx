// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
