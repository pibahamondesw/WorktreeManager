// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import type { InvokeArgs } from "@tauri-apps/api/core";
import { EditorView } from "@codemirror/view";
import { StrictMode } from "react";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args?: InvokeArgs) => Promise<unknown>>(),
  listeners: new Map<string, Set<(event: { payload: unknown }) => void>>(),
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
    const handlers = mocks.listeners.get(event) ?? new Set();
    mocks.listeners.set(event, handlers.add(handler));
    return Promise.resolve(() => handlers.delete(handler));
  }),
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn().mockResolvedValue(() => undefined) }),
}));
vi.mock("../../observeResize", () => import("../../test/fakeObserveResize"));

import { ChatWorkspace } from "./ChatWorkspace";
import { TaskConversationsContext } from "./taskConversationContext";
import type { SubagentLaunch, TabConversation } from "../../services/chat";
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

type Tab = {
  id: string;
  agent: string;
  conversationId: string | null;
  launchedBy?: TabConversation;
  subagents?: SubagentLaunch[];
};
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

const emit = (event: string, payload: unknown) =>
  act(() => mocks.listeners.get(event)?.forEach((handler) => handler({ payload })));

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

it("leads a subagent's conversation back to the chat that launched it", async () => {
  const parent = { taskId: "t0", agent: "codex" as const, tab: "main", conversationId: "th0" };
  backend([
    { id: "main", agent: "claude", conversationId: "c1" },
    { id: "a1", agent: "claude", conversationId: "c2", launchedBy: parent },
  ]);
  const open = vi.fn(() => true);
  const label = (taskId: string) => (taskId === "t0" ? "WOR-0" : null);
  render(
    <TaskConversationsContext.Provider value={{ label, open }}>
      <ChatWorkspace taskId="t1" agent="claude" folders={["/wt/a"]} />
    </TaskConversationsContext.Provider>
  );
  const tabs = await screen.findAllByRole("tab");
  expect(screen.queryByText(/Subagent of/)).toBeNull();
  fireEvent.click(tabs[1]);
  const back = await screen.findByRole("button", { name: "Open parent conversation" });
  const bar = back.parentElement!;
  expect(bar).toHaveTextContent(/Subagent of\s*WOR-0/);
  expect(bar.querySelector("svg.agent-logo-openai")).not.toBeNull();
  fireEvent.click(back);
  expect(open).toHaveBeenCalledWith(parent);
});

it("shows the subagents a tab's conversation launched, however their command ran", async () => {
  const launched = { taskId: "t2", agent: "codex" as const, tab: "a1", conversationId: "th2" };
  backend([{ id: "main", agent: "claude", conversationId: "c1", subagents: [launched] }]);
  const open = vi.fn(() => true);
  const label = (taskId: string) => (taskId === "t2" ? "WOR-2" : null);
  render(
    <TaskConversationsContext.Provider value={{ label, open }}>
      <ChatWorkspace taskId="t1" agent="claude" folders={["/wt/a"]} />
    </TaskConversationsContext.Provider>
  );
  const card = await screen.findByRole("region", { name: "Subagent: WOR-2" });
  fireEvent.click(within(card).getByRole("button", { name: "Open conversation" }));
  expect(open).toHaveBeenCalledWith(launched);
});

it("drops the way back once the launching task is gone", async () => {
  const parent = { taskId: "gone", agent: "codex" as const, tab: "main", conversationId: "th0" };
  backend([{ id: "main", agent: "claude", conversationId: "c1", launchedBy: parent }]);
  render(
    <TaskConversationsContext.Provider value={{ label: () => null, open: vi.fn() }}>
      <ChatWorkspace taskId="t1" agent="claude" folders={["/wt/a"]} />
    </TaskConversationsContext.Provider>
  );
  await screen.findByRole("tab");
  expect(screen.queryByText(/Subagent of/)).toBeNull();
});

it("reloads its tabs when the backend reports them changed", async () => {
  const tabs: Tab[] = [{ id: "main", agent: "claude", conversationId: null }];
  backend(tabs);
  renderWorkspace();
  await screen.findByRole("tab");
  tabs.push({ id: "cli", agent: "codex", conversationId: null });
  emit("chat-tabs", { taskId: "other" });
  emit("chat-tabs", { taskId: "t1" });
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
    emit("chat-status", {
      sessionId: "s",
      taskId: "t1",
      agent: "codex",
      origin: "user",
      status: { kind },
    });
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
  emit("chat-status", {
    sessionId: "s",
    taskId: "t1",
    agent: "codex",
    tab: "a1",
    origin: "user",
    status: { kind: "busy" },
    waiting: true,
    backgroundAgents: false,
  });
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

const REVIEW_STEPS = [
  {
    id: "review",
    label: "Review the diff",
    next: "fix",
    onFail: "fail",
    kind: {
      agent: {
        agent: { id: "codex", model: "default", effort: null },
        skill: null,
        prompt: "Review",
        onReject: null,
      },
    },
  },
  {
    id: "fix",
    label: "Fix findings",
    next: null,
    onFail: "fail",
    kind: {
      agent: {
        agent: { id: "codex", model: "default", effort: null },
        skill: null,
        prompt: "Fix",
        onReject: null,
      },
    },
  },
];

const automationChat = (sessionId: string, taskId: string, kind = "busy") => ({
  sessionId,
  taskId,
  agent: "codex",
  origin: "automation",
  status: { kind },
  waiting: false,
  backgroundAgents: false,
});

const attempt = (step: string, sessionId: string) => ({
  step,
  attempt: 1,
  status: "running",
  session_id: sessionId,
  conversation_id: null,
});

interface AutomationBackend {
  taskId: string;
  tabs?: Tab[];
  sessions?: Sessions;
  chats?: ReturnType<typeof automationChat>[] | string;
  item?: { status: string; phase: string; steps: ReturnType<typeof attempt>[] };
  automations?: string;
  takeOver?: { status: string; conversation_id: string | null } | string;
}

function automationBackend({
  taskId,
  tabs = [{ id: "main", agent: "claude", conversationId: null }],
  sessions = {},
  chats = [automationChat("as1", taskId)],
  item = { status: "active", phase: "review", steps: [attempt("review", "as1")] },
  automations,
  takeOver = { status: "paused", conversation_id: "conv1" },
}: AutomationBackend) {
  backend(tabs, sessions);
  const chatBackend = mocks.invoke.getMockImplementation()!;
  const run = { id: "r1", status: "running", items: [{ id: "i1", title: null, ...item }] };
  mocks.invoke.mockImplementation((command, invoked) => {
    switch (command) {
      case "chat_list":
        return typeof chats === "string" ? Promise.reject(chats) : Promise.resolve(chats);
      case "automations_list":
        return automations
          ? Promise.reject(automations)
          : Promise.resolve([
              { name: "nightly", definition: { steps: REVIEW_STEPS }, errors: [], warnings: [] },
            ]);
      case "automation_runs_list":
        return Promise.resolve({ runs: [run], next_before: null });
      case "automation_take_over":
        return typeof takeOver === "string"
          ? Promise.reject(takeOver)
          : Promise.resolve({ ...takeOver, step: "review", task_id: taskId });
      default:
        return chatBackend(command, invoked);
    }
  });
  return run;
}

const tabTitles = () => screen.getAllByRole("tab").map((tab) => tab.textContent);

it("opens a task without its own conversation on its automation's session, read-only", async () => {
  automationBackend({ taskId: "auto-default" });
  renderWorkspace({ taskId: "auto-default" });
  const tab = await screen.findByRole("tab", { name: "nightly · Review the diff" });
  expect(tab).toHaveAttribute("aria-selected", "true");
  expect(tabTitles()).toEqual(["New conversation", "nightly · Review the diff"]);
  expect(tab).toHaveAttribute("title", "nightly · Review the diff · Codex · Automation");
  expect(screen.getByTitle("Agent working")).toBeInTheDocument();
  await waitFor(() =>
    expect(chatOpens()).toEqual([
      expect.objectContaining({ taskId: "auto-default", agent: "codex", sessionId: "as1" }),
    ])
  );
  expect(screen.getByRole("note")).toHaveTextContent(
    "This conversation belongs to the nightly automation. Closing the tab does not stop it."
  );
  expect(screen.queryByRole("textbox")).toBeNull();
  expect(mocks.invoke).not.toHaveBeenCalledWith("chat_changes", expect.anything());
});

it("keeps the owner's conversation selected when the task has one", async () => {
  automationBackend({
    taskId: "auto-owned",
    tabs: [{ id: "main", agent: "claude", conversationId: "c1" }],
    sessions: { claude: [{ id: "c1", title: "Fix the login" }] },
  });
  renderWorkspace({ taskId: "auto-owned" });
  await screen.findByRole("tab", { name: "nightly · Review the diff" });
  expect(screen.getByRole("tab", { name: "Fix the login" })).toHaveAttribute(
    "aria-selected",
    "true"
  );
  expect(chatOpens()).toEqual([expect.objectContaining({ agent: "claude", tab: undefined })]);
});

it("hides a closed automation tab without stopping its session", async () => {
  automationBackend({ taskId: "auto-close" });
  renderWorkspace({ taskId: "auto-close" });
  await screen.findByRole("tab", { name: "nightly · Review the diff" });
  fireEvent.click(
    screen.getByRole("button", { name: "Close conversation nightly · Review the diff" })
  );
  await waitFor(() => expect(tabTitles()).toEqual(["New conversation"]));
  expect(screen.getByRole("tab")).toHaveAttribute("aria-selected", "true");
  const commands = mocks.invoke.mock.calls.map(([command]) => command);
  expect(commands).not.toContain("chat_stop");
  expect(commands).not.toContain("chat_tab_close");
});

it("names a shared session after the step its newest turn works on", async () => {
  const run = automationBackend({ taskId: "auto-shared" });
  renderWorkspace({ taskId: "auto-shared" });
  await screen.findByRole("tab", { name: "nightly · Review the diff" });
  run.items[0].steps = [attempt("review", "as1"), attempt("fix", "as1")];
  emit("chat-status", automationChat("as1", "auto-shared", "idle"));
  emit("chat-status", automationChat("as1", "auto-shared", "busy"));
  expect(await screen.findByRole("tab", { name: "nightly · Fix findings" })).toBeInTheDocument();
});

it("shows a session that appears later and drops one that ends while another is shown", async () => {
  automationBackend({ taskId: "auto-later", chats: [] });
  renderWorkspace({ taskId: "auto-later" });
  await screen.findByRole("tab", { name: "New conversation" });
  emit("chat-status", automationChat("as1", "other-task"));
  emit("chat-status", { ...automationChat("as2", "auto-later"), origin: "user" });
  emit("chat-status", automationChat("as1", "auto-later"));
  expect(await screen.findByRole("tab", { name: "nightly · Review the diff" })).toHaveAttribute(
    "aria-selected",
    "false"
  );
  emit("chat-status", automationChat("as1", "auto-later", "exited"));
  await waitFor(() => expect(tabTitles()).toEqual(["New conversation"]));
});

it("keeps the shown automation tab once its session ends, with no restart", async () => {
  automationBackend({ taskId: "auto-ended" });
  renderWorkspace({ taskId: "auto-ended" });
  await screen.findByRole("tab", { name: "nightly · Review the diff" });
  emit("chat-status", automationChat("as1", "auto-ended", "exited"));
  expect(await screen.findByText("Session ended")).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "nightly · Review the diff" })).toHaveAttribute(
    "aria-selected",
    "true"
  );
  expect(screen.queryByRole("button", { name: "Restart chat" })).toBeNull();
});

it("names a session generically when its automation cannot be found", async () => {
  automationBackend({ taskId: "auto-unknown", automations: "definitions unreadable" });
  renderWorkspace({ taskId: "auto-unknown" });
  expect(await screen.findByRole("tab", { name: "Automation" })).toBeInTheDocument();
  expect(screen.getByRole("note")).toHaveTextContent("This conversation belongs to an automation.");
  expect(screen.queryByRole("button", { name: "Take over" })).toBeNull();
});

it("opens the task's own chat when its sessions cannot be listed", async () => {
  automationBackend({ taskId: "auto-unlisted", chats: "registry unavailable" });
  renderWorkspace({ taskId: "auto-unlisted" });
  await waitFor(() =>
    expect(chatOpens()).toEqual([expect.objectContaining({ agent: "claude", tab: undefined })])
  );
  expect(tabTitles()).toEqual(["New conversation"]);
});

it("takes over the automation's conversation as the owner's chat in its tab's place", async () => {
  automationBackend({
    taskId: "auto-take",
    sessions: { codex: [{ id: "conv1", title: "Review the login" }] },
  });
  renderWorkspace({ taskId: "auto-take" });
  await screen.findByRole("tab", { name: "nightly · Review the diff" });
  fireEvent.click(screen.getByRole("button", { name: "Take over" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("automation_take_over", {
      target: { name: "nightly", runId: "r1", item: "i1", by: "user" },
    })
  );
  await waitFor(() =>
    expect(chatOpens().at(-1)).toEqual(
      expect.objectContaining({
        agent: "codex",
        tab: "new1",
        mode: "restart",
        conversation: "conv1",
      })
    )
  );
  expect(await screen.findByRole("tab", { name: "Review the login" })).toHaveAttribute(
    "aria-selected",
    "true"
  );
  expect(tabTitles()).toEqual(["New conversation", "Review the login"]);
});

it("explains a take over whose step has not paused yet", async () => {
  automationBackend({
    taskId: "auto-stopping",
    takeOver: { status: "active", conversation_id: null },
  });
  renderWorkspace({ taskId: "auto-stopping" });
  fireEvent.click(await screen.findByRole("button", { name: "Take over" }));
  expect(
    await screen.findByText("The step is still stopping; it pauses once its turn ends")
  ).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "nightly · Review the diff" })).toBeInTheDocument();
});

it("shows why a take over failed", async () => {
  automationBackend({ taskId: "auto-refused", takeOver: "The item is not running" });
  renderWorkspace({ taskId: "auto-refused" });
  fireEvent.click(await screen.findByRole("button", { name: "Take over" }));
  expect(await screen.findByText("The item is not running")).toBeInTheDocument();
});

it("offers no take over for an item the Pipeline could not take over", async () => {
  automationBackend({
    taskId: "auto-done",
    item: { status: "completed", phase: "review", steps: [attempt("review", "as1")] },
  });
  renderWorkspace({ taskId: "auto-done" });
  await screen.findByRole("tab", { name: "nightly · Review the diff" });
  expect(screen.queryByRole("button", { name: "Take over" })).toBeNull();
});

it("opens on the automation when the task it showed last had no conversation of its own", async () => {
  automationBackend({ taskId: "auto-return", chats: [] });
  const first = renderWorkspace({ taskId: "auto-return" });
  await waitFor(() => expect(chatOpens()).toHaveLength(1));
  first.unmount();
  automationBackend({ taskId: "auto-return" });
  renderWorkspace({ taskId: "auto-return" });
  expect(await screen.findByRole("tab", { name: "nightly · Review the diff" })).toHaveAttribute(
    "aria-selected",
    "true"
  );
});

it("offers no take over from a session the item's current step no longer runs", async () => {
  automationBackend({
    taskId: "auto-earlier-step",
    item: { status: "active", phase: "fix", steps: [attempt("review", "as1")] },
  });
  renderWorkspace({ taskId: "auto-earlier-step" });
  await screen.findByRole("tab", { name: "nightly · Review the diff" });
  expect(screen.queryByRole("button", { name: "Take over" })).toBeNull();
});

it("closes the automation tab when the step paused before its agent had a conversation", async () => {
  automationBackend({
    taskId: "auto-no-conversation",
    takeOver: { status: "paused", conversation_id: null },
  });
  renderWorkspace({ taskId: "auto-no-conversation" });
  fireEvent.click(await screen.findByRole("button", { name: "Take over" }));
  await waitFor(() => expect(tabTitles()).toEqual(["New conversation"]));
  expect(screen.getByRole("tab")).toHaveAttribute("aria-selected", "true");
  expect(mocks.invoke).not.toHaveBeenCalledWith("chat_tab_open", expect.anything());
});

const SIDE_REPLY = {
  ...IDLE_CHAT,
  items: [
    { id: "q", kind: "user", text: "What does run do?" },
    { id: "r", kind: "assistant", text: "It runs.\n\nTwice." },
  ],
};

function sideBackend() {
  const tabs: Tab[] = [
    { id: "main", agent: "claude", conversationId: "c1" },
    { id: "a1", agent: "claude", conversationId: "c2" },
  ];
  backend(tabs);
  const fallback = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, invoked) => {
    const tab = (invoked as { tab?: string } | undefined)?.tab ?? "";
    if (command === "chat_open" && tab.startsWith("side-")) return Promise.resolve(SIDE_REPLY);
    return fallback(command, invoked);
  });
  return tabs;
}

const editorOf = (box: HTMLElement) => EditorView.findFromDOM(box)!;
const composerText = (box: HTMLElement) => editorOf(box).state.doc.toString();
const typeInto = (box: HTMLElement, value: string) =>
  act(() =>
    editorOf(box).dispatch({
      changes: { from: 0, to: editorOf(box).state.doc.length, insert: value },
      selection: { anchor: value.length },
    })
  );

const mainComposer = () => screen.getAllByRole("textbox", { name: "Message Claude Code" })[0];

const askAside = async (question: string) => {
  const box = mainComposer();
  await waitFor(() => expect(box).toHaveAttribute("contenteditable", "true"));
  await typeInto(box, `/btw ${question}`);
  fireEvent.keyDown(box, { key: "Enter" });
  return screen.findByRole("complementary", { name: "Side chat" });
};

const sideOpens = () =>
  chatOpens().filter((args) => String((args as { tab?: string }).tab).startsWith("side-")) as {
    tab: string;
  }[];

it("asks a side question in a side chat that knows the conversation without writing to it", async () => {
  sideBackend();
  renderWorkspace({ taskId: "side-ask" });
  await screen.findAllByRole("textbox", { name: "Message Claude Code" });
  await askAside("What does run do?");

  await waitFor(() => expect(sideOpens()).toHaveLength(1));
  const [opened] = sideOpens();
  expect(opened).toMatchObject({
    taskId: "side-ask",
    agent: "claude",
    mode: "fresh",
    asideOf: "main",
  });
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("chat_send", {
      taskId: "side-ask",
      agent: "claude",
      text: "What does run do?",
      images: [],
      tab: opened.tab,
    })
  );
  expect(mocks.invoke).not.toHaveBeenCalledWith(
    "chat_send",
    expect.objectContaining({ tab: undefined })
  );
  expect(composerText(mainComposer())).toBe("");

  await typeInto(mainComposer(), "Also check this:");
  fireEvent.click(await screen.findByRole("button", { name: "Add reply to main chat" }));
  await waitFor(() =>
    expect(composerText(mainComposer())).toBe("Also check this:\n\n> It runs.\n>\n> Twice.")
  );

  fireEvent.click(screen.getByRole("button", { name: "Close side chat" }));
  expect(screen.queryByRole("complementary", { name: "Side chat" })).toBeNull();
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("chat_stop", {
      taskId: "side-ask",
      agent: "claude",
      tab: opened.tab,
    })
  );
});

it("opens a fresh side chat for each question and stops the one it replaces", async () => {
  sideBackend();
  renderWorkspace({ taskId: "side-again" });
  await screen.findAllByRole("textbox", { name: "Message Claude Code" });
  await askAside("first");
  await waitFor(() => expect(sideOpens()).toHaveLength(1));
  const [first] = sideOpens();
  await askAside("second");
  await waitFor(() => expect(sideOpens()).toHaveLength(2));
  expect(sideOpens()[1].tab).not.toBe(first.tab);
  expect(mocks.invoke).toHaveBeenCalledWith("chat_stop", {
    taskId: "side-again",
    agent: "claude",
    tab: first.tab,
  });
});

it.each([
  { action: "close", launchSucceeds: true },
  { action: "replace", launchSucceeds: true },
  { action: "close", launchSucceeds: false },
])(
  "stops a pending side chat on $action (launch succeeds: $launchSucceeds)",
  async ({ action, launchSucceeds }) => {
    sideBackend();
    let finishLaunch!: () => void;
    const launch = new Promise<void>((resolve) => {
      finishLaunch = resolve;
    });
    const live = new Set<string>();
    const fallback = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(async (command, args) => {
      const tab = (args as { tab?: string } | undefined)?.tab;
      if (typeof tab === "string" && tab.startsWith("side-")) {
        if (command === "chat_open") {
          if (sideOpens().length === 1) {
            await launch;
            if (!launchSucceeds) throw new Error("Could not start the agent");
          }
          live.add(tab);
        }
        if (command === "chat_stop") live.delete(tab);
      }
      return fallback(command, args);
    });

    const taskId = `side-pending-${action}-${launchSucceeds}`;
    renderWorkspace({ taskId });
    await screen.findByRole("textbox", { name: "Message Claude Code" });
    await askAside("first");
    const [first] = sideOpens();
    expect(first).toBeDefined();

    if (action === "close") {
      fireEvent.click(screen.getByRole("button", { name: "Close side chat" }));
      expect(screen.queryByRole("complementary", { name: "Side chat" })).toBeNull();
    } else {
      await askAside("second");
      expect(sideOpens()).toHaveLength(2);
    }

    expect(mocks.invoke).not.toHaveBeenCalledWith("chat_stop", expect.anything());
    await act(async () => finishLaunch());

    expect([...live]).toEqual(
      sideOpens()
        .slice(1)
        .map(({ tab }) => tab)
    );
    expect(mocks.invoke).toHaveBeenCalledWith("chat_stop", {
      taskId,
      agent: "claude",
      tab: first.tab,
    });
    expect(mocks.invoke).not.toHaveBeenCalledWith(
      "chat_send",
      expect.objectContaining({ tab: first.tab })
    );
  }
);

it("blocks adding a side reply until the queued edit is finished", async () => {
  sideBackend();
  const fallback = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, args) => {
    const tab = (args as { tab?: string } | undefined)?.tab;
    if (command === "chat_open" && !tab) {
      return Promise.resolve({
        ...IDLE_CHAT,
        status: { kind: "busy" },
        items: [{ id: "q", kind: "user", text: "Queued note", status: "queued" }],
      });
    }
    if (command === "chat_hold_queued") return Promise.resolve("Queued note");
    return fallback(command, args);
  });
  renderWorkspace({ taskId: "side-queued-edit" });
  await screen.findByRole("textbox", { name: "Message Claude Code" });
  await askAside("about main");
  const addReply = await screen.findByRole("button", { name: "Add reply to main chat" });
  await waitFor(() => expect(addReply).toBeEnabled());
  await typeInto(mainComposer(), "Keep this draft");

  fireEvent.click(screen.getByRole("button", { name: "Edit queued message" }));
  const edit = await screen.findByRole("textbox", { name: "Queued message" });
  expect(addReply).toBeDisabled();
  expect(
    screen.getByText("Finish editing the queued message before adding the reply.")
  ).toBeVisible();
  fireEvent.click(addReply);
  expect(composerText(edit)).toBe("Queued note");

  fireEvent.keyDown(edit, { key: "Escape" });
  await waitFor(() => expect(addReply).toBeEnabled());
  expect(
    screen.queryByText("Finish editing the queued message before adding the reply.")
  ).not.toBeInTheDocument();
  expect(composerText(mainComposer())).toBe("Keep this draft");
  fireEvent.click(addReply);
  await waitFor(() =>
    expect(composerText(mainComposer())).toBe("Keep this draft\n\n> It runs.\n>\n> Twice.")
  );
});

it("shows the side chat only beside its conversation and stops it with that tab", async () => {
  sideBackend();
  renderWorkspace({ taskId: "side-tabs" });
  await screen.findAllByRole("textbox", { name: "Message Claude Code" });
  await askAside("about main");
  await waitFor(() => expect(sideOpens()).toHaveLength(1));
  const [opened] = sideOpens();

  fireEvent.click(screen.getAllByRole("tab")[1]);
  await waitFor(() =>
    expect(screen.queryByRole("complementary", { name: "Side chat" })).toBeNull()
  );
  expect(mocks.invoke).not.toHaveBeenCalledWith("chat_stop", expect.anything());
  fireEvent.click(screen.getAllByRole("tab")[0]);
  await screen.findByRole("complementary", { name: "Side chat" });

  fireEvent.click(screen.getAllByRole("button", { name: /^Close conversation/ })[0]);
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("chat_stop", {
      taskId: "side-tabs",
      agent: "claude",
      tab: opened.tab,
    })
  );
  expect(screen.queryByRole("complementary", { name: "Side chat" })).toBeNull();
});

it("stops the side chat when a tab reload confirms its source was closed from outside", async () => {
  const tabs = sideBackend();
  const fallback = mocks.invoke.getMockImplementation()!;
  let tabsUnavailable = false;
  mocks.invoke.mockImplementation((command, args) => {
    if (command === "chat_tabs") {
      return tabsUnavailable ? Promise.reject("Could not load tabs") : Promise.resolve([...tabs]);
    }
    return fallback(command, args);
  });
  const view = renderWorkspace({ taskId: "side-external-close" });
  await screen.findByRole("textbox", { name: "Message Claude Code" });
  fireEvent.click(screen.getAllByRole("tab")[1]);
  await askAside("about the other tab");
  await waitFor(() => expect(sideOpens()).toHaveLength(1));
  const [opened] = sideOpens();

  tabs.pop();
  tabsUnavailable = true;
  emit("chat-tabs", { taskId: "side-external-close" });
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not load tabs");
  expect(screen.getByRole("complementary", { name: "Side chat" })).toBeInTheDocument();
  expect(mocks.invoke).not.toHaveBeenCalledWith("chat_stop", expect.anything());

  view.unmount();
  renderWorkspace({ taskId: "side-external-close" });
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not load tabs");
  expect(screen.getByRole("tab")).toHaveAttribute("data-tab", "main");
  expect(mocks.invoke).not.toHaveBeenCalledWith("chat_stop", expect.anything());

  tabsUnavailable = false;
  emit("chat-tabs", { taskId: "side-external-close" });
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("chat_stop", {
      taskId: "side-external-close",
      agent: "claude",
      tab: opened.tab,
    })
  );
  expect(mocks.invoke.mock.calls.filter(([command]) => command === "chat_stop")).toHaveLength(1);
  expect(screen.queryByRole("complementary", { name: "Side chat" })).toBeNull();
  expect(screen.getByRole("tab")).toHaveAttribute("data-tab", "main");
});

it("keeps a side chat opened in a new tab while another tab closes", async () => {
  sideBackend();
  let finishClose!: () => void;
  const closing = new Promise<void>((resolve) => {
    finishClose = resolve;
  });
  const fallback = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, args) =>
    command === "chat_tab_close" ? closing : fallback(command, args)
  );

  renderWorkspace({ taskId: "side-concurrent-close" });
  const [main] = await screen.findAllByRole("tab");
  fireEvent.click(main);
  fireEvent.click(screen.getAllByRole("button", { name: /^Close conversation/ })[1]);
  fireEvent.click(screen.getByRole("button", { name: "New Claude Code conversation" }));
  await screen.findByRole("tab", { name: "New conversation", selected: true });
  await askAside("about the new tab");
  await act(async () => finishClose());

  expect(screen.getAllByRole("tab").map((tab) => tab.getAttribute("data-tab"))).toEqual([
    "main",
    "new1",
  ]);
  expect(screen.getByRole("complementary", { name: "Side chat" })).toBeInTheDocument();
  expect(mocks.invoke).not.toHaveBeenCalledWith("chat_stop", expect.anything());
});

it("completes /btw from the command menu so the question can follow", async () => {
  sideBackend();
  renderWorkspace({ taskId: "side-menu" });
  const box = (await screen.findAllByRole("textbox", { name: "Message Claude Code" }))[0];
  await waitFor(() => expect(box).toHaveAttribute("contenteditable", "true"));
  await typeInto(box, "/bt");
  fireEvent.keyDown(box, { key: "Tab" });
  expect(composerText(box)).toBe("/btw ");
  expect(screen.queryByRole("complementary", { name: "Side chat" })).toBeNull();
});

it("asks the side question once even when the panel mounts twice while opening", async () => {
  sideBackend();
  render(
    <StrictMode>
      <ChatWorkspace taskId="side-strict" agent="claude" folders={["/wt/a"]} />
    </StrictMode>
  );
  await screen.findAllByRole("textbox", { name: "Message Claude Code" });
  await askAside("still asked?");
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith(
      "chat_send",
      expect.objectContaining({ text: "still asked?" })
    )
  );
  expect(mocks.invoke.mock.calls.filter(([command]) => command === "chat_send")).toHaveLength(1);
});
