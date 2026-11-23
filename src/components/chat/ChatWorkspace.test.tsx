// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import type { InvokeArgs } from "@tauri-apps/api/core";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args?: InvokeArgs) => Promise<unknown>>(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: InvokeArgs) =>
    invokeWithAgentCatalog(mocks.invoke, command, args),
  Channel: class {
    onmessage?: (m: unknown) => void;
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(event, handler);
    return Promise.resolve(() => mocks.listeners.delete(event));
  }),
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn().mockResolvedValue(() => undefined) }),
}));
vi.mock("../../observeResize", () => import("../../test/fakeObserveResize"));

import { ChatWorkspace } from "./ChatWorkspace";
import { invokeWithAgentCatalog } from "../../services/agents.fixture";

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

type Tab = { id: string; agent: string; conversationId: string | null };
type Sessions = Record<string, { id: string; title: string }[] | string>;

function backend(tabs: Tab[], sessions: Sessions = {}) {
  let opened = 0;
  mocks.invoke.mockImplementation((command: string, invoked?: InvokeArgs) => {
    const args = invoked as Record<string, unknown> | undefined;
    switch (command) {
      case "chat_tabs":
        return Promise.resolve(tabs);
      case "chat_open":
        return Promise.resolve(IDLE_CHAT);
      case "chat_list":
        return Promise.resolve([]);
      case "chat_tab_open":
        return Promise.resolve({ id: `new${++opened}`, agent: args?.agent });
      case "chat_sessions": {
        const listed = sessions[args?.agent as string] ?? [];
        return typeof listed === "string"
          ? Promise.reject(listed)
          : Promise.resolve({
              sessions: listed.map((session) => ({ ...session, updatedAt: 1, current: false })),
              canRename: true,
              canArchive: false,
            });
      }
      default:
        return Promise.resolve(undefined);
    }
  });
}

const chatOpens = (): unknown[] =>
  mocks.invoke.mock.calls
    .filter(([command]) => command === "chat_open")
    .map(([, args]) => args as unknown);

const renderWorkspace = (props: Partial<Parameters<typeof ChatWorkspace>[0]> = {}) =>
  render(<ChatWorkspace taskId="t1" agent="claude" folders={["/wt/a"]} {...props} />);

beforeEach(() => mocks.invoke.mockReset());
afterEach(cleanup);

it("shows each open conversation as a tab with its agent's logo and title", async () => {
  backend(
    [
      { id: "main", agent: "claude", conversationId: "c1" },
      { id: "a1", agent: "codex", conversationId: "th1" },
      { id: "a2", agent: "claude", conversationId: null },
    ],
    { claude: [{ id: "c1", title: "Fix the login" }], codex: [{ id: "th1", title: "Review PR" }] }
  );
  renderWorkspace();
  const tablist = await screen.findByRole("tablist", { name: "Open conversations" });
  await waitFor(() =>
    expect(
      within(tablist)
        .getAllByRole("tab")
        .map((tab) => tab.textContent)
    ).toEqual(["Fix the login", "Review PR", "New conversation"])
  );
  const logos = [...tablist.querySelectorAll("svg[class*='agent-logo']")];
  expect(logos.map((logo) => logo.classList[0])).toEqual([
    "agent-logo-claude",
    "agent-logo-openai",
    "agent-logo-claude",
  ]);
  expect(screen.getByRole("tab", { name: "Fix the login" })).toHaveAttribute(
    "aria-selected",
    "true"
  );
  await waitFor(() =>
    expect(chatOpens()).toEqual([expect.objectContaining({ agent: "claude", tab: undefined })])
  );

  fireEvent.click(screen.getByRole("tab", { name: "Review PR" }));
  await waitFor(() =>
    expect(chatOpens().at(-1)).toEqual(expect.objectContaining({ agent: "codex", tab: "a1" }))
  );
});

it("opens a new conversation in a tab of the task's agent", async () => {
  backend([{ id: "main", agent: "claude", conversationId: "c1" }]);
  renderWorkspace();
  fireEvent.click(await screen.findByRole("button", { name: "New Claude Code conversation" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("chat_tab_open", {
      taskId: "t1",
      mainAgent: "claude",
      agent: "claude",
    })
  );
  expect(await screen.findByRole("tab", { name: "New conversation" })).toHaveAttribute(
    "aria-selected",
    "true"
  );
  await waitFor(() =>
    expect(chatOpens().at(-1)).toEqual(
      expect.objectContaining({ agent: "claude", tab: "new1", mode: "attach" })
    )
  );
});

it("closes a tab, shows its neighbor and offers a new conversation once none is open", async () => {
  backend([
    { id: "main", agent: "claude", conversationId: null },
    { id: "a1", agent: "codex", conversationId: null },
  ]);
  renderWorkspace();
  const tabs = await screen.findAllByRole("tab");
  fireEvent.click(tabs[0]);
  const [closeMain] = screen.getAllByRole("button", { name: /^Close conversation/ });
  fireEvent.click(closeMain);
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("chat_tab_close", {
      taskId: "t1",
      mainAgent: "claude",
      tab: "main",
    })
  );
  await waitFor(() => expect(screen.getAllByRole("tab")).toHaveLength(1));
  expect(screen.getByRole("tab")).toHaveAttribute("data-tab", "a1");
  expect(screen.getByRole("tab")).toHaveAttribute("aria-selected", "true");

  fireEvent.click(screen.getByRole("button", { name: /^Close conversation/ }));
  expect(await screen.findByText("No conversations are open.")).toBeInTheDocument();
});

it("renames a tab's conversation by double-clicking it", async () => {
  backend([{ id: "a1", agent: "claude", conversationId: "c1" }], {
    claude: [{ id: "c1", title: "Fix the login" }],
  });
  renderWorkspace();
  fireEvent.doubleClick(await screen.findByRole("tab", { name: "Fix the login" }));
  const name = screen.getByRole("textbox", { name: "Conversation name" });
  fireEvent.change(name, { target: { value: "Login fix" } });
  fireEvent.submit(name.closest("form")!);
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("chat_session_rename", {
      taskId: "t1",
      agent: "claude",
      folders: ["/wt/a"],
      id: "c1",
      title: "Login fix",
      tab: "a1",
    })
  );
});

it("lists only conversations not open in tabs, and opens a picked one in a new tab", async () => {
  backend([{ id: "main", agent: "claude", conversationId: "c1" }], {
    claude: [
      { id: "c1", title: "Current work" },
      { id: "c0", title: "Older work" },
    ],
    codex: "codex app-server crashed",
  });
  renderWorkspace();
  await screen.findByRole("tab", { name: "Current work" });
  fireEvent.click(screen.getByRole("button", { name: "Conversations" }));
  const list = await screen.findByRole("list", { name: "Conversations" });
  expect(within(list).queryByText("Current work")).toBeNull();
  expect(screen.getByText("Codex: codex app-server crashed")).toBeInTheDocument();

  fireEvent.click(within(list).getByRole("button", { name: /Older work/ }));
  await waitFor(() =>
    expect(chatOpens().at(-1)).toEqual(
      expect.objectContaining({ agent: "claude", tab: "new1", mode: "restart", conversation: "c0" })
    )
  );
  expect(screen.queryByRole("dialog", { name: "Conversations" })).toBeNull();
});

it("reloads its tabs when the backend reports them changed", async () => {
  const tabs: Tab[] = [{ id: "main", agent: "claude", conversationId: null }];
  backend(tabs);
  renderWorkspace();
  await screen.findByRole("tab");
  tabs.push({ id: "cli", agent: "codex", conversationId: null });
  act(() => mocks.listeners.get("chat-tabs")?.({ payload: { taskId: "other" } }));
  act(() => mocks.listeners.get("chat-tabs")?.({ payload: { taskId: "t1" } }));
  await waitFor(() => expect(screen.getAllByRole("tab")).toHaveLength(2));
  expect(screen.getAllByRole("tab")[0]).toHaveAttribute("aria-selected", "true");
});

it("shows a slow title load even when the chat keeps reporting its status meanwhile", async () => {
  backend([{ id: "main", agent: "codex", conversationId: "th1" }]);
  const answers: ((value: unknown) => void)[] = [];
  const answered = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, args) =>
    command === "chat_sessions"
      ? new Promise((resolve) => answers.push(resolve))
      : answered(command, args)
  );
  renderWorkspace({ agent: "codex" });
  await screen.findByRole("tab", { name: "Untitled conversation" });
  const status = (kind: string) =>
    act(() =>
      mocks.listeners.get("chat-status")?.({
        payload: { sessionId: "s", taskId: "t1", agent: "codex", origin: "user", status: { kind } },
      })
    );
  status("busy");
  status("idle");
  status("idle");
  status("idle");
  await waitFor(() => expect(answers).toHaveLength(2));
  act(() =>
    answers[0]({
      sessions: [{ id: "th1", title: "Hello!", updatedAt: 1, current: true }],
      canRename: true,
      canArchive: true,
    })
  );
  expect(await screen.findByRole("tab", { name: "Hello!" })).toBeInTheDocument();
});

it("marks a tab whose agent needs input", async () => {
  backend([
    { id: "main", agent: "claude", conversationId: null },
    { id: "a1", agent: "codex", conversationId: null },
  ]);
  renderWorkspace();
  await screen.findAllByRole("tab");
  act(() =>
    mocks.listeners.get("chat-status")?.({
      payload: {
        sessionId: "s",
        taskId: "t1",
        agent: "codex",
        tab: "a1",
        origin: "user",
        status: { kind: "busy" },
        waiting: true,
        backgroundAgents: false,
      },
    })
  );
  expect(await screen.findByTitle("Agent needs your input")).toBeInTheDocument();
});

it("falls back to the main tab when the tabs cannot be read", async () => {
  backend([]);
  mocks.invoke.mockImplementation((command: string) =>
    command === "chat_tabs"
      ? Promise.reject("disk full")
      : Promise.resolve(command === "chat_open" ? IDLE_CHAT : undefined)
  );
  renderWorkspace({ agent: "codex" });
  expect(await screen.findByRole("alert")).toHaveTextContent("disk full");
  await waitFor(() =>
    expect(chatOpens()).toEqual([expect.objectContaining({ agent: "codex", tab: undefined })])
  );
});
