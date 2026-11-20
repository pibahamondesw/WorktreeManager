// @vitest-environment jsdom
import { StrictMode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import type { invoke, InvokeArgs } from "@tauri-apps/api/core";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn<typeof invoke>(),
  channels: { opened: 0 },
  term: {
    open: vi.fn(),
    write: vi.fn(),
    reset: vi.fn(),
    focus: vi.fn(),
    dispose: vi.fn(),
    loadAddon: vi.fn(),
    onData: vi.fn(() => ({ dispose: vi.fn() })),
    options: {} as Record<string, unknown>,
    cols: 80,
    rows: 24,
  },
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: InvokeArgs) =>
    invokeWithAgentCatalog(mocks.invoke, command, args),
  Channel: class {
    onmessage?: (e: unknown) => void;
    id = ++mocks.channels.opened;
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn().mockResolvedValue(() => undefined) }),
}));
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
import { invokeWithAgentCatalog } from "../../services/agents.fixture";
import { requestChatIntent } from "../../embedded/chatIntents";
import { Terminal } from "@xterm/xterm";
import { applyTypography, defaultTypography } from "../../appearance";

const IDLE_CHAT = {
  generation: 1,
  status: { kind: "idle" },
  items: [],
  pending: [],
  controls: {
    models: [],
    model: null,
    effort: null,
    modes: [],
    mode: null,
    commands: [],
    todos: [],
    context: null,
  },
};

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

function renderView(onBack = vi.fn(), onSwitchSurface = vi.fn()) {
  render(
    <TaskView
      task={task}
      surface={{ kind: "terminal", agent: "claude" }}
      sidebarCollapsed={false}
      onExpandSidebar={vi.fn()}
      onBack={onBack}
      onSwitchSurface={onSwitchSurface}
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
      onSwitchSurface: vi.fn(),
    };
    const view = render(<TaskView {...props} surface={{ kind: "terminal", agent: "claude" }} />);
    expect(await screen.findByRole("img", { name: "running" })).toBeInTheDocument();
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
    expect(mocks.invoke).toHaveBeenCalledWith(
      "terminal_detach",
      expect.objectContaining({ taskId: "t1", agent: "claude" })
    );
    expect(mocks.invoke.mock.calls.some(([command]) => command === "terminal_close")).toBe(false);
    view.unmount();
    expect(mocks.invoke).toHaveBeenCalledWith(
      "terminal_detach",
      expect.objectContaining({ taskId: "t1", agent: "codex" })
    );
  });

  it("keeps a remounted terminal attached when the discarded mount's open resolves late", async () => {
    const opens: ((result: unknown) => void)[] = [];
    mocks.invoke.mockImplementation((command: string) =>
      command === "terminal_open"
        ? new Promise((resolve) => opens.push(resolve))
        : Promise.resolve(undefined)
    );
    const view = render(
      <StrictMode>
        <TaskView
          task={task}
          surface={{ kind: "terminal", agent: "codex" }}
          sidebarCollapsed={false}
          onExpandSidebar={vi.fn()}
          onBack={vi.fn()}
          onSwitchSurface={vi.fn()}
        />
      </StrictMode>
    );
    await waitFor(() => expect(opens).toHaveLength(2));
    const detaches = () =>
      mocks.invoke.mock.calls.filter(([command]) => command === "terminal_detach");
    const [discarded, mounted] = mocks.invoke.mock.calls
      .filter(([command]) => command === "terminal_open")
      .map(([, args]) => (args as { onEvent: { id: number } }).onEvent.id);
    expect(detaches()).toEqual([]);
    const opened = { created: false, status: { kind: "running" }, replay: "" };
    opens[1](opened);
    expect(await screen.findByRole("img", { name: "running" })).toBeInTheDocument();
    opens[0](opened);
    await waitFor(() =>
      expect(detaches()).toEqual([
        ["terminal_detach", { taskId: "t1", agent: "codex", attachment: discarded }],
      ])
    );
    view.unmount();
    expect(detaches()).toContainEqual([
      "terminal_detach",
      { taskId: "t1", agent: "codex", attachment: mounted },
    ]);
  });

  it("stops the terminal before switching the agent to its chat", async () => {
    let finishStop: (() => void) | undefined;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "terminal_stop")
        return new Promise<void>((resolve) => (finishStop = resolve));
      return Promise.resolve({ created: true, status: { kind: "running" }, replay: "" });
    });
    const onSwitchSurface = vi.fn();
    renderView(vi.fn(), onSwitchSurface);
    expect(await screen.findByRole("img", { name: "running" })).toBeInTheDocument();
    const view = screen.getByRole("group", { name: "View" });
    fireEvent.click(within(view).getByRole("button", { name: "Chat" }));
    expect(mocks.invoke).toHaveBeenCalledWith("terminal_stop", { taskId: "t1", agent: "claude" });
    expect(within(view).getByRole("button", { name: "Terminal" })).toBeDisabled();
    expect(onSwitchSurface).not.toHaveBeenCalled();
    finishStop?.();
    await waitFor(() =>
      expect(onSwitchSurface).toHaveBeenCalledWith({ kind: "chat", agent: "claude" })
    );
    expect(mocks.invoke.mock.calls.some(([command]) => command === "terminal_close")).toBe(false);
  });

  it("keeps the current view and shows the error when stopping fails", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "terminal_stop"
        ? Promise.reject("could not stop")
        : Promise.resolve({ created: true, status: { kind: "running" }, replay: "" })
    );
    const onSwitchSurface = vi.fn();
    renderView(vi.fn(), onSwitchSurface);
    expect(await screen.findByRole("img", { name: "running" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Chat" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("could not stop");
    expect(onSwitchSurface).not.toHaveBeenCalled();
  });

  it("hands the chat to the other agent after stopping the current one", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open"
        ? Promise.resolve({
            generation: 1,
            status: { kind: "idle" },
            items: [{ id: "u", kind: "user", text: "Plan it" }],
            pending: [],
            controls: {
              models: [],
              model: null,
              effort: null,
              modes: [],
              mode: null,
              commands: [],
              todos: [],
              context: null,
            },
          })
        : Promise.resolve(undefined)
    );
    const onSwitchSurface = vi.fn();
    const props = {
      task,
      sidebarCollapsed: false,
      onExpandSidebar: vi.fn(),
      onBack: vi.fn(),
      onSwitchSurface,
    };
    const { rerender } = render(
      <TaskView {...props} surface={{ kind: "chat", agent: "claude" }} />
    );
    await screen.findByText("Plan it");
    fireEvent.click(await screen.findByRole("button", { name: "Agent: Claude Code" }));
    fireEvent.mouseDown(within(screen.getByRole("listbox", { name: "Agent" })).getByText("Codex"));
    fireEvent.click(screen.getByRole("button", { name: "Continue in Codex" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_stop", { taskId: "t1", agent: "claude" })
    );
    expect(mocks.invoke).toHaveBeenCalledWith("chat_tab_set_agent", {
      taskId: "t1",
      mainAgent: "claude",
      tab: "main",
      agent: "codex",
    });
    await waitFor(() =>
      expect(onSwitchSurface).toHaveBeenCalledWith({ kind: "chat", agent: "codex" })
    );
    rerender(<TaskView {...props} surface={{ kind: "chat", agent: "codex" }} />);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "chat_open",
        expect.objectContaining({ agent: "codex", mode: "fresh", handoffFrom: "claude" })
      )
    );
  });

  it("opens a conversation another view asked for in a tab, once", async () => {
    let opened = 0;
    mocks.invoke.mockImplementation((command: string, args?: InvokeArgs) => {
      if (command === "chat_open") return Promise.resolve(IDLE_CHAT);
      if (command === "chat_tab_open")
        return Promise.resolve({ id: `tab${++opened}`, agent: (args as { agent: string }).agent });
      return Promise.resolve(undefined);
    });
    const props = {
      task,
      sidebarCollapsed: false,
      onExpandSidebar: vi.fn(),
      onBack: vi.fn(),
      onSwitchSurface: vi.fn(),
    };
    const chatOpens = () => mocks.invoke.mock.calls.filter(([command]) => command === "chat_open");
    render(<TaskView {...props} surface={{ kind: "chat", agent: "claude" }} />);
    await waitFor(() => expect(chatOpens()).toHaveLength(1));

    requestChatIntent("t1", { agent: "codex", mode: "restart", conversation: "c-codex" });
    await waitFor(() =>
      expect(chatOpens().at(-1)?.[1]).toEqual(
        expect.objectContaining({
          agent: "codex",
          tab: "tab1",
          mode: "restart",
          conversation: "c-codex",
        })
      )
    );
    expect(mocks.invoke).toHaveBeenCalledWith("chat_tab_open", {
      taskId: "t1",
      mainAgent: "claude",
      agent: "codex",
    });
    await waitFor(() =>
      expect(props.onSwitchSurface).toHaveBeenCalledWith({ kind: "chat", agent: "codex" })
    );

    fireEvent.click(screen.getByRole("tab", { name: "New conversation" }));
    requestChatIntent("t1", { agent: "codex", mode: "restart", conversation: "c-codex" });
    await waitFor(() =>
      expect(screen.getByRole("tab", { selected: true })).toHaveAttribute("data-tab", "tab1")
    );
    expect(opened).toBe(1);
  });

  it("ends the agent's chat process before returning to the task list", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open"
        ? Promise.resolve({
            generation: 1,
            status: { kind: "idle" },
            items: [],
            pending: [],
            controls: {
              models: [],
              model: null,
              effort: null,
              modes: [],
              mode: null,
              commands: [],
              todos: [],
              context: null,
            },
          })
        : Promise.resolve(
            command === "chat_list"
              ? [
                  {
                    taskId: "t1",
                    agent: "codex",
                    tab: "a1",
                    origin: "user",
                    status: { kind: "idle" },
                  },
                ]
              : undefined
          )
    );
    const onBack = vi.fn();
    render(
      <TaskView
        task={task}
        surface={{ kind: "chat", agent: "codex" }}
        sidebarCollapsed={false}
        onExpandSidebar={vi.fn()}
        onBack={onBack}
        onSwitchSurface={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Close editor" }));
    await waitFor(() => expect(onBack).toHaveBeenCalledOnce());
    expect(mocks.invoke).toHaveBeenCalledWith("chat_stop", {
      taskId: "t1",
      agent: "codex",
      tab: "a1",
    });
    expect(mocks.invoke.mock.calls.some(([command]) => command === "chat_close")).toBe(false);
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
        onSwitchSurface={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Close editor" }));
    expect(mocks.invoke).toHaveBeenCalledWith("vscode_close", { taskId: task.id });
    expect(onBack).not.toHaveBeenCalled();
    finishClose?.();
    await waitFor(() => expect(onBack).toHaveBeenCalledOnce());
  });
  it("opens every task folder in OpenCode and closes its server before leaving", async () => {
    let finishClose: (() => void) | undefined;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "opencode_open") return new Promise(() => undefined);
      if (command === "opencode_close")
        return new Promise<void>((resolve) => {
          finishClose = resolve;
        });
      return Promise.resolve();
    });
    const onBack = vi.fn();
    render(
      <TaskView
        task={task}
        surface={{ kind: "opencode" }}
        sidebarCollapsed={false}
        onExpandSidebar={vi.fn()}
        onBack={onBack}
        onSwitchSurface={vi.fn()}
      />
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("opencode_open", {
        taskId: task.id,
        folders: ["/wt/a", "/wt/b"],
      })
    );
    fireEvent.click(screen.getByRole("button", { name: "Close editor" }));
    expect(mocks.invoke).toHaveBeenCalledWith("opencode_close", { taskId: task.id });
    expect(onBack).not.toHaveBeenCalled();
    finishClose?.();
    await waitFor(() => expect(onBack).toHaveBeenCalledOnce());
  });

  it("shows the task header and attaches to the task terminal", async () => {
    mocks.invoke.mockResolvedValue({ created: true, status: { kind: "running" }, replay: "old" });
    renderView();
    expect(screen.getByText("WOR-87")).toBeInTheDocument();
    expect(screen.getByText("Embedded agent")).toBeInTheDocument();
    expect(await screen.findByRole("img", { name: "running" })).toBeInTheDocument();
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

  it("renders the terminal with the terminal font and follows its changes", async () => {
    mocks.invoke.mockResolvedValue({ created: true, status: { kind: "running" }, replay: "" });
    const typography = defaultTypography();
    applyTypography({ ...typography, code: { family: "Fira Code", size: 13 } });
    renderView();
    await screen.findByRole("img", { name: "running" });
    expect(vi.mocked(Terminal)).toHaveBeenLastCalledWith(
      expect.objectContaining({ fontFamily: '"Fira Code", Menlo, Monaco, monospace', fontSize: 12 })
    );
    mocks.invoke.mockClear();

    applyTypography({ ...typography, terminal: { family: "Iosevka", size: 16 } });

    expect(mocks.term.options).toMatchObject({
      fontFamily: "Iosevka, Menlo, Monaco, monospace",
      fontSize: 16,
    });
    expect(mocks.invoke).toHaveBeenCalledWith(
      "terminal_resize",
      expect.objectContaining({ cols: 80, rows: 24 })
    );
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
