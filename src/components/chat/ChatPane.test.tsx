// @vitest-environment jsdom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { EditorView } from "@codemirror/view";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  catalog: [] as unknown[],
  channels: [] as { onmessage?: (m: unknown) => void }[],
  dropHandlers: new Set<(event: { payload: unknown }) => void>(),
  markdownRenders: [] as unknown[],
  windows: false,
}));

vi.mock("react-markdown", async (importOriginal) => {
  const { createElement } = await import("react");
  const actual = await importOriginal<typeof import("react-markdown")>();
  return {
    ...actual,
    default: (props: Parameters<typeof actual.default>[0]) => {
      mocks.markdownRenders.push(props.children);
      return createElement(actual.default, props);
    },
  };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: unknown) =>
    command === "agents_catalog"
      ? Promise.resolve(mocks.catalog)
      : (mocks.invoke(command, args) as Promise<unknown>),
  Channel: class {
    onmessage?: (m: unknown) => void;
    constructor() {
      mocks.channels.push(this);
    }
  },
}));
vi.mock("../../platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../platform")>()),
  get IS_WINDOWS() {
    return mocks.windows;
  },
}));
vi.mock("../../observeResize", () => import("../../test/fakeObserveResize"));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (handler: (event: { payload: unknown }) => void) => {
      mocks.dropHandlers.add(handler);
      return Promise.resolve(() => mocks.dropHandlers.delete(handler));
    },
  }),
}));

const dropFiles = async (paths: string[], position = { x: 0, y: 0 }) => {
  const box = screen.getByRole("textbox");
  await waitFor(() => expect(box).toHaveAttribute("contenteditable", "true"));
  document.elementFromPoint = (x: number, y: number) => (x === 0 && y === 0 ? box : document.body);
  await act(() => {
    for (const handler of mocks.dropHandlers) {
      handler({ payload: { type: "enter", paths, position } });
      handler({ payload: { type: "drop", paths, position } });
    }
  });
};

import { ChatPane } from "./ChatPane";
import { FirstPromptContext } from "./firstPromptContext";
import { describeAgent, loadAgentCatalog } from "../../services/agents";
import { AGENT_CATALOG } from "../../services/agents.fixture";
import { resize } from "../../test/fakeObserveResize";
import { layOutScroller } from "../../test/fakeLayout";

const editorOf = (box: HTMLElement) => EditorView.findFromDOM(box)!;
const composerText = (box: HTMLElement) => editorOf(box).state.doc.toString();
const typeInto = (box: HTMLElement, value: string) =>
  act(() =>
    editorOf(box).dispatch({
      changes: { from: 0, to: editorOf(box).state.doc.length, insert: value },
      selection: { anchor: value.length },
    })
  );

const snapshot = (overrides = {}) => ({
  generation: 3,
  status: { kind: "idle" },
  items: [
    { id: "u", kind: "user", text: "Fix the bug" },
    { id: "a", kind: "assistant", text: "Done with **care**" },
    {
      id: "f",
      kind: "fileChange",
      text: "/wt/a/x.ts",
      title: "Edited 1 file",
      detail: "--- /wt/a/x.ts\n+added\n-removed",
      status: "completed",
    },
  ],
  pending: [],
  controls: {
    models: [
      { id: "opus", label: "Opus", efforts: ["low", "high"], images: true },
      { id: "haiku", label: "Haiku", efforts: [], images: false },
    ],
    model: "opus",
    effort: "high",
    modes: [
      { id: "default", label: "Ask before edits", description: "" },
      { id: "plan", label: "Plan", description: "" },
    ],
    mode: "default",
    commands: [
      { name: "model", description: "Choose the model", action: { kind: "model" } },
      { name: "plan", description: "Plan first", action: { kind: "mode", mode: "plan" } },
      {
        name: "review",
        description: "Review changes",
        action: { kind: "insert", text: "/review " },
      },
      { name: "status", description: "Show plan usage", action: { kind: "status" } },
      { name: "hooks", description: "Review hooks", action: { kind: "hooks" } },
    ],
    todos: [
      { text: "Read the code", status: "completed" },
      { text: "Write the fix", status: "inProgress" },
    ],
    context: { used: 50, max: 200 },
    usage: null,
  },
  ...overrides,
});

const emit = (generation: number, event: unknown) =>
  act(() => mocks.channels.at(-1)!.onmessage?.({ generation, events: [event] }));

beforeAll(async () => {
  mocks.catalog = AGENT_CATALOG;
  await loadAgentCatalog();
});

beforeEach(() => {
  mocks.catalog = AGENT_CATALOG;
  mocks.channels.length = 0;
  mocks.invoke.mockReset().mockImplementation((command: string) => {
    if (command === "chat_open") return Promise.resolve(snapshot());
    return Promise.resolve(undefined);
  });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("ChatPane", () => {
  it.each([
    ["Rewind code to here", "code"],
    ["Fork conversation and rewind code", "both"],
  ])("previews and confirms files after selecting %s", async (label, action) => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open")
        return Promise.resolve(
          snapshot({
            items: [{ id: "u", kind: "user", text: "Fix the bug", checkpoint: "checkpoint" }],
          })
        );
      if (command === "chat_rewind_preview")
        return Promise.resolve({ token: "confirmation", files: ["/wt/a/x.ts"] });
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    fireEvent.click(await screen.findByRole("button", { name: "Rewind or fork" }));
    fireEvent.click(screen.getByRole("menuitem", { name: label }));
    await screen.findByRole("list", { name: "Files to restore" });
    expect(mocks.invoke.mock.calls.some(([command]) => command === "chat_history")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Confirm restore" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_history", {
        taskId: "t1",
        agent: "codex",
        generation: 3,
        itemId: "u",
        action,
        confirmation: "confirmation",
      })
    );
  });

  it("forks from a user message and attaches to the replacement conversation", async () => {
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    fireEvent.click(await screen.findByRole("button", { name: "Rewind or fork" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Fork conversation from here" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_history", {
        taskId: "t1",
        agent: "codex",
        generation: 3,
        itemId: "u",
        action: "conversation",
        confirmation: undefined,
      })
    );
    await waitFor(() =>
      expect(mocks.invoke.mock.calls.filter(([command]) => command === "chat_open")).toHaveLength(2)
    );
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "chat_open",
      expect.objectContaining({ mode: "attach" })
    );
  });

  it("disables history actions while the agent works or waits for approval", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "chat_open" ? snapshot({ status: { kind: "busy" } }) : undefined)
    );
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    expect(await screen.findByRole("button", { name: "Rewind or fork" })).toBeDisabled();
  });

  it("does not reattach the wrong task when a fork completes after switching tasks", async () => {
    let finishFork: () => void = () => undefined;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "chat_history")
        return new Promise<void>((resolve) => {
          finishFork = resolve;
        });
      return Promise.resolve(undefined);
    });
    const view = render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    fireEvent.click(await screen.findByRole("button", { name: "Rewind or fork" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Fork conversation from here" }));
    view.rerender(<ChatPane taskId="t2" agent="claude" folders={["/wt/b"]} />);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "chat_open",
        expect.objectContaining({ taskId: "t2" })
      )
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await act(async () => finishFork());
    expect(mocks.invoke.mock.calls.filter(([command]) => command === "chat_open")).toHaveLength(2);
  });

  it("restores the transcript and expands file diffs", async () => {
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a", "/wt/b"]} />);
    await screen.findByText("Fix the bug");
    expect(mocks.invoke).toHaveBeenCalledWith(
      "chat_open",
      expect.objectContaining({
        taskId: "t1",
        agent: "codex",
        folders: ["/wt/a", "/wt/b"],
        mode: "attach",
      })
    );
    expect(screen.getByText("care").tagName).toBe("STRONG");
    expect(screen.queryByRole("button", { name: /Edited 1 file/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Made 1 edit" }));
    fireEvent.click(screen.getByRole("button", { name: /Edited 1 file/ }));
    expect(screen.getByText("+added")).toHaveClass("text-success");
  });

  it("renders only the messages near the viewport and follows new ones at the bottom", async () => {
    const items = Array.from({ length: 120 }, (_, index) => ({
      id: `m${index}`,
      kind: "assistant",
      text: `Message ${index}`,
    }));
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "chat_open" ? snapshot({ items }) : undefined)
    );
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    await screen.findByText("Message 119");
    expect(screen.getByText("Message 90")).toBeInTheDocument();
    expect(screen.queryByText("Message 89")).not.toBeInTheDocument();

    const scroller = screen.getByText("Message 119").closest(".chat-scroll") as HTMLElement;
    layOutScroller(scroller, 600, () => 120);
    resize();
    expect(screen.getByText("Message 110")).toBeInTheDocument();
    expect(screen.queryByText("Message 109")).not.toBeInTheDocument();
    expect(scroller.scrollTop).toBe(120 * 120 - 600);

    scroller.scrollTop = 0;
    fireEvent.scroll(scroller);
    expect(screen.getByText("Message 0")).toBeInTheDocument();
    expect(screen.queryByText("Message 119")).not.toBeInTheDocument();

    emit(3, { type: "upsert", item: { id: "m120", kind: "assistant", text: "Message 120" } });
    expect(scroller.scrollTop).toBe(0);
    expect(screen.queryByText("Message 120")).not.toBeInTheDocument();

    scroller.scrollTop = scroller.scrollHeight;
    fireEvent.scroll(scroller);
    emit(3, { type: "upsert", item: { id: "m121", kind: "assistant", text: "Message 121" } });
    expect(screen.getByText("Message 121")).toBeInTheDocument();
    expect(scroller.scrollTop).toBe(122 * 120 - 600);
  });

  it("loads earlier history at the top and keeps the first visible message in place", async () => {
    const message = (index: number) => ({
      id: `m${index}`,
      kind: "assistant",
      text: `Message ${index}`,
    });
    const range = (from: number, to: number) =>
      Array.from({ length: to - from }, (_, index) => message(from + index));
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "chat_open"
          ? snapshot({ sessionId: "s1", items: range(10, 30), earlier: 10 })
          : undefined
      )
    );
    const loads = () =>
      mocks.invoke.mock.calls.filter(([command]) => command === "chat_load_earlier");
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    await screen.findByText("Message 29");
    const scroller = screen.getByText("Message 29").closest(".chat-scroll") as HTMLElement;
    layOutScroller(scroller, 600, () => 120);
    resize();
    expect(screen.getByText("Loading earlier messages…")).toBeInTheDocument();
    expect(loads()).toEqual([]);

    scroller.scrollTop = 0;
    fireEvent.scroll(scroller);
    expect(loads()).toEqual([["chat_load_earlier", { sessionId: "s1", generation: 3 }]]);

    emit(3, { type: "earlier", items: range(0, 10), earlier: 0 });
    expect(scroller.scrollTop).toBe(10 * 120);
    expect(screen.getByText("Message 10")).toBeInTheDocument();
    expect(screen.queryByText("Loading earlier messages…")).not.toBeInTheDocument();
    expect(loads()).toHaveLength(1);
  });

  it("keeps subagent steps inside their card instead of the main conversation", async () => {
    const items = [
      { id: "u1", kind: "user", text: "Investigate" },
      { id: "a1", kind: "agent", text: "", title: "Find the bug", status: "inProgress" },
      { id: "a2", kind: "agent", text: "", title: "Read the docs", status: "inProgress" },
      { id: "s1", kind: "assistant", text: "Sub reply one", parent: "a1" },
      { id: "s2", kind: "assistant", text: "Sub reply two", parent: "a2" },
      { id: "t1", kind: "command", text: "ls", parent: "a1" },
      { id: "m1", kind: "assistant", text: "Main reply" },
    ];
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "chat_open" ? snapshot({ status: { kind: "busy" }, items }) : undefined
      )
    );
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    await screen.findByText("Main reply");
    expect(screen.queryByText("Sub reply one")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Ran 1 command/ })).not.toBeInTheDocument();

    const first = screen.getByRole("region", { name: "Subagent: Find the bug" });
    fireEvent.click(within(first).getAllByRole("button")[0]);
    expect(within(first).getByText("Sub reply one")).toBeInTheDocument();
    expect(within(first).queryByText("Sub reply two")).not.toBeInTheDocument();
    expect(
      within(screen.getByRole("region", { name: "Subagent: Read the docs" })).getByText("1 step")
    ).toBeInTheDocument();
  });

  it("keeps reasoning dropdowns separate from expandable tool steps", async () => {
    const items = [
      { id: "u", kind: "user", text: "Investigate" },
      {
        id: "r",
        kind: "reasoning",
        text: "Checking options",
        status: "completed",
        durationMs: 5000,
      },
      { id: "c", kind: "command", text: "ls", status: "completed" },
      { id: "t", kind: "tool", text: "Read file", status: "failed" },
      { id: "a", kind: "assistant", text: "Here is the answer" },
      { id: "live", kind: "reasoning", text: "", status: "inProgress" },
    ];
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "chat_open" ? snapshot({ status: { kind: "busy" }, items }) : undefined
      )
    );
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    await screen.findByText("Here is the answer");
    expect(screen.getByText("Thinking…")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Thinking…" })).not.toBeInTheDocument();
    expect(screen.queryByText("Checking options")).not.toBeInTheDocument();
    expect(screen.queryByText("ls")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Thought for 5s" }));
    expect(screen.getByText("Checking options")).toBeVisible();
    expect(screen.queryByText("ls")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Ran 1 command · Used 1 tool · 1 failed" }));
    expect(screen.getByText("ls")).toBeVisible();
    expect(screen.getByText("Read file")).toBeVisible();
  });

  it("shows the message while the turn is prepared and restores it if sending fails", async () => {
    const sends: { resolve: () => void; reject: (e: string) => void }[] = [];
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(snapshot({ items: [] }));
      if (command === "chat_send")
        return new Promise<void>((resolve, reject) => sends.push({ resolve, reject }));
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Codex" });
    typeInto(box, "Add tests");
    fireEvent.keyDown(box, { key: "Enter" });

    await waitFor(() => expect(sends).toHaveLength(1));
    expect(composerText(box)).toBe("");
    expect(screen.getByLabelText("Sending message")).toHaveTextContent("Add tests");
    expect(screen.getByRole("status")).toHaveTextContent("Preparing…");
    typeInto(box, "again");
    fireEvent.keyDown(box, { key: "Enter" });
    expect(sends).toHaveLength(1);

    emit(3, { type: "status", status: { kind: "busy" } });
    await act(async () => sends[0].resolve());
    expect(screen.queryByText("Preparing…")).toBeNull();
    expect(screen.getByText("Codex is working…")).toBeInTheDocument();
    expect(screen.getByLabelText("Sending message")).toBeInTheDocument();
    emit(3, { type: "upsert", item: { id: "u1", kind: "user", text: "Add tests" } });
    expect(screen.queryByLabelText("Sending message")).toBeNull();

    emit(3, { type: "status", status: { kind: "idle" } });
    typeInto(box, "");
    typeInto(box, "Fails");
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(sends).toHaveLength(2));
    await act(async () => sends[1].reject("Agent unavailable"));
    expect(composerText(box)).toBe("Fails");
    expect(screen.queryByLabelText("Sending message")).toBeNull();
    expect(screen.getByText("Agent unavailable")).toBeInTheDocument();
  });

  it("re-renders only the streamed row when a delta arrives", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open")
        return Promise.resolve(
          snapshot({
            status: { kind: "busy" },
            items: [
              ...snapshot().items,
              { id: "q", kind: "user", text: "Queued note", status: "queued" },
            ],
          })
        );
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    await screen.findByText("Queued note");
    emit(3, { type: "upsert", item: { id: "b", kind: "assistant", text: "Work" } });
    mocks.markdownRenders.length = 0;

    emit(3, { type: "delta", id: "b", field: "text", delta: "ing" });

    expect(screen.getByText("Working")).toBeInTheDocument();
    expect(mocks.markdownRenders).toEqual(["Working"]);
  });

  it("fills a new conversation's empty composer with the first prompt once", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "chat_open" ? snapshot({ items: [] }) : undefined)
    );
    const firstPrompt = vi.fn().mockResolvedValue("Work on ENG-7");
    const pane = () => (
      <FirstPromptContext.Provider value={firstPrompt}>
        <ChatPane taskId="seeded" agent="claude" folders={["/wt/a"]} />
      </FirstPromptContext.Provider>
    );
    const { unmount } = render(pane());
    const box = await screen.findByRole("textbox", { name: "Message Claude Code" });
    await waitFor(() => expect(composerText(box)).toBe("Work on ENG-7"));
    expect(firstPrompt).toHaveBeenCalledExactlyOnceWith("seeded");

    typeInto(box, "");
    unmount();
    render(pane());
    const reopened = await screen.findByRole("textbox", { name: "Message Claude Code" });
    expect(composerText(reopened)).toBe("");
    expect(firstPrompt).toHaveBeenCalledOnce();
  });

  it("offers the same first prompt again to the conversation /clear starts", async () => {
    const clearCommand = { name: "clear", description: "Start fresh", action: { kind: "clear" } };
    mocks.invoke.mockImplementation((command: string, args: { mode?: string }) =>
      Promise.resolve(
        command === "chat_open"
          ? snapshot({
              generation: args.mode === "fresh" ? 4 : 3,
              items: [],
              controls: { ...snapshot().controls, commands: [clearCommand] },
            })
          : undefined
      )
    );
    const firstPrompt = vi.fn().mockResolvedValue("Work on ENG-7");
    render(
      <FirstPromptContext.Provider value={firstPrompt}>
        <ChatPane taskId="cleared" agent="claude" folders={["/wt/a"]} />
      </FirstPromptContext.Provider>
    );
    const box = await screen.findByRole("textbox", { name: "Message Claude Code" });
    await waitFor(() => expect(composerText(box)).toBe("Work on ENG-7"));
    typeInto(box, "/clear");
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "chat_open",
        expect.objectContaining({ mode: "fresh" })
      )
    );
    await waitFor(() => expect(composerText(box)).toBe("Work on ENG-7"));
    expect(firstPrompt).toHaveBeenCalledTimes(2);
  });

  it("leaves the composer alone when the conversation already has messages", async () => {
    const firstPrompt = vi.fn().mockResolvedValue("Work on ENG-7");
    render(
      <FirstPromptContext.Provider value={firstPrompt}>
        <ChatPane taskId="ongoing" agent="claude" folders={["/wt/a"]} />
      </FirstPromptContext.Provider>
    );
    const box = await screen.findByRole("textbox", { name: "Message Claude Code" });
    await screen.findByText("Fix the bug");
    expect(composerText(box)).toBe("");
    expect(firstPrompt).not.toHaveBeenCalled();
  });

  it("sends on Enter, streams the reply and ignores a stale generation", async () => {
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Claude Code" });
    typeInto(box, "Add tests");
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_send", {
        taskId: "t1",
        agent: "claude",
        text: "Add tests",
        images: [],
      })
    );
    await waitFor(() => expect(composerText(box)).toBe(""));

    emit(3, { type: "status", status: { kind: "busy" } });
    emit(3, { type: "upsert", item: { id: "b", kind: "assistant", text: "Work" } });
    emit(3, { type: "delta", id: "b", field: "text", delta: "ing" });
    emit(2, { type: "delta", id: "b", field: "text", delta: " STALE" });
    expect(screen.getByText("Working")).toBeInTheDocument();

    typeInto(box, "and add docs");
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_send", {
        taskId: "t1",
        agent: "claude",
        text: "and add docs",
        images: [],
      })
    );
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(mocks.invoke).toHaveBeenCalledWith("chat_interrupt", { taskId: "t1", agent: "claude" });
  });

  it("attaches pasted and dropped images as removable thumbnails and sends them", async () => {
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Claude Code" });
    const png = (name: string) => new File(["hi"], name, { type: "image/png" });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "chat_read_image")
        return Promise.resolve({ name: "two.png", mediaType: "image/png", data: "aGk=" });
      return Promise.resolve(undefined);
    });
    fireEvent.paste(box, { clipboardData: { files: [png("one.png")] } });
    await dropFiles(["/shots/two.png"]);
    expect(mocks.invoke).toHaveBeenCalledWith("chat_read_image", { path: "/shots/two.png" });
    const thumbnails = await screen.findByRole("list", { name: "Attached images" });
    await waitFor(() => expect(within(thumbnails).getAllByRole("img")).toHaveLength(2));
    fireEvent.click(screen.getByRole("button", { name: "Remove one.png" }));
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_send", {
        taskId: "t1",
        agent: "claude",
        text: "",
        images: [{ mediaType: "image/png", data: "aGk=" }],
      })
    );
    expect(screen.queryByRole("list", { name: "Attached images" })).not.toBeInTheDocument();
  });

  it("inserts dropped non-image files as quoted @ mentions at the caret", async () => {
    render(<ChatPane taskId="drop-mentions" agent="claude" folders={["/wt/a"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Claude Code" });
    await typeInto(box, "read");
    await dropFiles(["/docs/My Notes.md", "/wt/a/spec.pdf"]);
    expect(composerText(box)).toBe('read @"/docs/My Notes.md" @/wt/a/spec.pdf ');

    await dropFiles(["/elsewhere.txt"], { x: 50, y: 50 });
    expect(composerText(box)).toBe('read @"/docs/My Notes.md" @/wt/a/spec.pdf ');
    expect(mocks.invoke).not.toHaveBeenCalledWith("chat_read_image", expect.anything());
  });

  it("reports dropped images that cannot be read", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "chat_read_image")
        return Promise.reject("Images must be 10 MB or smaller: huge.png");
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    await screen.findByRole("textbox", { name: "Message Claude Code" });
    await dropFiles(["/shots/huge.png"]);
    expect(await screen.findByRole("alert")).toHaveTextContent("10 MB or smaller: huge.png");
    expect(screen.queryByRole("list", { name: "Attached images" })).not.toBeInTheDocument();
  });

  it("skips unsupported image types and reports oversized ones", async () => {
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Claude Code" });
    const huge = new File(["x"], "huge.jpg", { type: "image/jpeg" });
    Object.defineProperty(huge, "size", { value: 10 * 1024 * 1024 + 1 });
    fireEvent.paste(box, {
      clipboardData: {
        files: [new File(["<svg/>"], "a.svg", { type: "image/svg+xml" }), huge],
      },
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("10 MB or smaller: huge.jpg");
    expect(screen.queryByRole("list", { name: "Attached images" })).not.toBeInTheDocument();
  });

  it("ignores images when the model does not accept them", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open"
        ? Promise.resolve(snapshot({ controls: { ...snapshot().controls, model: "haiku" } }))
        : Promise.resolve(undefined)
    );
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Codex" });
    expect(box.getAttribute("aria-placeholder")).not.toContain("paste images");
    fireEvent.paste(box, {
      clipboardData: {
        files: [new File(["hi"], "one.png", { type: "image/png" })],
        getData: () => "",
      },
    });
    await dropFiles(["/shots/two.png"]);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByRole("list", { name: "Attached images" })).not.toBeInTheDocument();
    expect(composerText(box)).toBe("@/shots/two.png ");
  });

  const pasteText = (box: HTMLElement, text: string) =>
    fireEvent.paste(box, {
      clipboardData: { files: [], getData: (type: string) => (type === "text/plain" ? text : "") },
    });
  const readyComposer = async () => {
    Object.assign(Range.prototype, {
      getClientRects: () => [],
      getBoundingClientRect: () => new DOMRect(),
    });
    const box = await screen.findByRole("textbox", { name: "Message Claude Code" });
    await waitFor(() => expect(box).not.toHaveAttribute("aria-disabled"));
    return box;
  };
  const largeLog = "error: something failed\n".repeat(1400);

  it("attaches pastes of 32 KB or more as removable text and sends them after the message", async () => {
    render(<ChatPane taskId="paste-1" agent="claude" folders={["/wt/a"]} />);
    const box = await readyComposer();
    pasteText(box, "short note");
    expect(composerText(box)).toBe("short note");
    pasteText(box, largeLog);
    pasteText(box, "```js\n" + largeLog + "```");
    expect(composerText(box)).toBe("short note");
    const chips = screen.getByRole("list", { name: "Attached text" });
    expect(within(chips).getAllByRole("listitem")[0]).toHaveTextContent(
      "Pasted text · 1,400 lines · 33 KB"
    );
    expect(within(chips).getAllByRole("listitem")[0].title).toContain("32 KB or more");
    fireEvent.click(screen.getAllByRole("button", { name: /^Remove Pasted text/ })[0]);
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_send", {
        taskId: "paste-1",
        agent: "claude",
        text: `short note\n\nPasted text · 1,402 lines · 33 KB:\n\`\`\`\`\n\`\`\`js\n${largeLog}\`\`\`\n\`\`\`\``,
        images: [],
      })
    );
    expect(screen.queryByRole("list", { name: "Attached text" })).not.toBeInTheDocument();
  });

  it("sends a text attachment alone instead of running it as a slash command", async () => {
    render(<ChatPane taskId="paste-2" agent="claude" folders={["/wt/a"]} />);
    const box = await readyComposer();
    pasteText(box, largeLog);
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_send", {
        taskId: "paste-2",
        agent: "claude",
        text: `Pasted text · 1,400 lines · 33 KB:\n\`\`\`\n${largeLog}\n\`\`\``,
        images: [],
      })
    );
  });

  it("restores text attachments when sending fails", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "chat_send") return Promise.reject("agent is gone");
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="paste-3" agent="claude" folders={["/wt/a"]} />);
    const box = await readyComposer();
    pasteText(box, largeLog);
    fireEvent.keyDown(box, { key: "Enter" });
    expect(await screen.findByRole("alert")).toHaveTextContent("agent is gone");
    expect(screen.getByRole("list", { name: "Attached text" })).toBeInTheDocument();
  });

  it("pastes the clipboard inline with Command-Shift-V on macOS", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "chat_clipboard_text") return Promise.resolve(largeLog);
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="paste-4" agent="claude" folders={["/wt/a"]} />);
    const box = await readyComposer();
    await typeInto(box, "log: ");
    fireEvent.keyDown(box, { key: "V", metaKey: true, shiftKey: true });
    await waitFor(() => expect(composerText(box)).toBe(`log: ${largeLog}`));
    expect(screen.queryByRole("list", { name: "Attached text" })).not.toBeInTheDocument();
  });

  it("reports a clipboard that cannot be read for an inline paste", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "chat_clipboard_text") return Promise.reject("Could not read the clipboard");
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="paste-5" agent="claude" folders={["/wt/a"]} />);
    const box = await readyComposer();
    fireEvent.keyDown(box, { key: "V", metaKey: true, shiftKey: true });
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not read the clipboard");
  });

  it("keeps the native Ctrl-Shift-V paste inline on Windows", async () => {
    mocks.windows = true;
    try {
      render(<ChatPane taskId="paste-6" agent="claude" folders={["/wt/a"]} />);
      const box = await readyComposer();
      fireEvent.keyDown(box, { key: "V", ctrlKey: true, shiftKey: true });
      pasteText(box, largeLog);
      expect(composerText(box)).toBe(largeLog);
      fireEvent.keyDown(box, { key: "a" });
      pasteText(box, largeLog);
      expect(screen.getByRole("list", { name: "Attached text" })).toBeInTheDocument();
      expect(mocks.invoke).not.toHaveBeenCalledWith("chat_clipboard_text", undefined);
    } finally {
      mocks.windows = false;
    }
  });

  it("keeps elicitation validation errors in the original form and removes it on external resolution", async () => {
    const response = { elicitation: { action: "accept", content: { download: false } } };
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open")
        return Promise.resolve(
          snapshot({
            status: { kind: "busy" },
            pending: [
              {
                id: "elicitation:1",
                title: "MCP · browser",
                kind: "elicitation",
                format: "text",
                serverName: "browser",
                mode: "form",
                detail: "Choose a download",
                requestedSchema: {
                  type: "object",
                  required: ["download"],
                  properties: { download: { type: "boolean", title: "Download" } },
                },
              },
            ],
          })
        );
      if (command === "chat_respond")
        return Promise.reject("Form does not satisfy the requested schema");
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    fireEvent.change(await screen.findByRole("combobox", { name: "Download" }), {
      target: { value: "false" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Accept" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Form does not satisfy the requested schema"
    );
    expect(screen.getByRole("combobox", { name: "Download" })).toHaveValue("false");
    expect(mocks.invoke).toHaveBeenCalledWith("chat_respond", {
      taskId: "t1",
      agent: "codex",
      requestId: "elicitation:1",
      generation: 3,
      response,
    });
    expect(mocks.invoke).not.toHaveBeenCalledWith("chat_send", expect.anything());
    emit(3, { type: "resolved", id: "elicitation:1" });
    expect(screen.queryByRole("button", { name: "Accept" })).not.toBeInTheDocument();
  });

  it("answers approvals and questions through chat_respond", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open"
        ? Promise.resolve(
            snapshot({
              status: { kind: "busy" },
              pending: [
                {
                  id: "r1",
                  title: "Run command",
                  detail: "$ rm -rf build",
                  format: "text",
                  kind: "approval",
                  decisions: [
                    { id: "accept", label: "Approve" },
                    { id: "decline", label: "Reject" },
                  ],
                },
                {
                  id: "q1",
                  title: "Codex has a question",
                  format: "text",
                  kind: "question",
                  questions: [
                    {
                      id: "db",
                      header: "DB",
                      question: "Which database?",
                      options: [{ label: "Postgres" }],
                      allowOther: true,
                      multiSelect: false,
                      secret: false,
                    },
                  ],
                },
              ],
            })
          )
        : Promise.resolve(undefined)
    );
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    await screen.findByText("$ rm -rf build");
    fireEvent.click(screen.getByRole("button", { name: /Reject$/ }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_respond", {
        generation: 3,
        taskId: "t1",
        agent: "codex",
        requestId: "r1",
        response: { decision: "decline" },
      })
    );

    const answer = screen.getByRole("button", { name: "Answer" });
    expect(answer).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Postgres" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Answer: Which database?" }), {
      target: { value: "with pgvector" },
    });
    fireEvent.click(answer);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_respond", {
        generation: 3,
        taskId: "t1",
        agent: "codex",
        requestId: "q1",
        response: { answers: { db: ["Postgres", "with pgvector"] } },
      })
    );
  });

  it("keeps the input disabled until the session finishes starting", async () => {
    let resolveOpen: (value: unknown) => void = () => undefined;
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open"
        ? new Promise((resolve) => (resolveOpen = resolve))
        : Promise.resolve(undefined)
    );
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    const box = screen.getByRole("textbox", { name: "Message Claude Code" });
    expect(box).toHaveAttribute("aria-disabled", "true");
    expect(box).toHaveAttribute("aria-placeholder", "Starting Claude Code…");

    await act(async () => resolveOpen(snapshot()));
    expect(box).not.toHaveAttribute("aria-disabled");
    expect(box).toHaveFocus();
  });

  it.each([
    [
      "claude",
      "Claude Code is working — messages you send now run after this turn unless you steer them (Esc to stop)",
    ],
    [
      "codex",
      "Codex is working — messages you send now run after this turn unless you steer them (Esc to stop)",
    ],
  ] as const)("tells how %s handles messages sent mid-turn", async (agent, placeholder) => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open"
        ? Promise.resolve(snapshot({ status: { kind: "busy" } }))
        : Promise.resolve(undefined)
    );
    render(<ChatPane taskId="t1" agent={agent} folders={["/wt/a"]} />);
    const box = screen.getByRole("textbox", { name: `Message ${describeAgent(agent).label}` });
    await waitFor(() => expect(box).toHaveAttribute("aria-placeholder", placeholder));
  });

  it("tells that messages sent mid-turn wait when the agent cannot steer", async () => {
    mocks.catalog = AGENT_CATALOG.map((agent) => ({
      ...agent,
      capabilities: { ...agent.capabilities, steer: false },
    }));
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open"
        ? Promise.resolve(snapshot({ status: { kind: "busy" } }))
        : Promise.resolve(undefined)
    );
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    const box = screen.getByRole("textbox", { name: "Message Claude Code" });
    await waitFor(() =>
      expect(box).toHaveAttribute(
        "aria-placeholder",
        "Claude Code is working — messages you send now run after this turn (Esc to stop)"
      )
    );
  });

  const busyWithQueue = () =>
    snapshot({
      status: { kind: "busy" },
      items: [
        { id: "u", kind: "user", text: "Fix the bug" },
        { id: "s", kind: "user", text: "Use merge instead", status: "steered" },
        { id: "i", kind: "notice", text: "Interrupted" },
        { id: "q", kind: "user", text: "Then run the tests", status: "queued" },
      ],
    });

  it("lets a queued message steer the turn or be removed, and marks steered ones", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open" ? Promise.resolve(busyWithQueue()) : Promise.resolve(undefined)
    );
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    await screen.findByText("Queued");
    expect(screen.getByText("Steered")).toBeInTheDocument();
    expect(screen.getByText("Interrupted")).toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: "Steer" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_steer_queued", {
        taskId: "t1",
        agent: "claude",
        itemId: "q",
      })
    );
    fireEvent.click(screen.getByRole("button", { name: "Remove queued message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_unqueue", {
        taskId: "t1",
        agent: "claude",
        itemId: "q",
      })
    );
  });

  const openQueuedEdit = async (taskId: string) => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(busyWithQueue());
      if (command === "chat_hold_queued") return Promise.resolve("Then run the tests");
      return Promise.resolve(undefined);
    });
    const view = render(<ChatPane taskId={taskId} agent="claude" folders={["/wt/a"]} />);
    const draft = screen.getByRole("textbox", { name: "Message Claude Code" });
    await waitFor(() => expect(draft).toHaveAttribute("contenteditable", "true"));
    await typeInto(draft, "Half a thought");
    fireEvent.click(await screen.findByRole("button", { name: "Edit queued message" }));
    const box = await screen.findByRole("textbox", { name: "Queued message" });
    await waitFor(() => expect(composerText(box)).toBe("Then run the tests"));
    expect(screen.getByText("Editing")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit queued message" })).toBeNull();
    return { view, box };
  };

  const released = (taskId: string, edit: unknown) =>
    waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_release_queued", {
        taskId,
        agent: "claude",
        itemId: "q",
        edit,
      })
    );

  it("edits a queued message in the composer and gives the draft back once saved", async () => {
    const { box } = await openQueuedEdit("edit-save");
    expect(mocks.invoke).toHaveBeenCalledWith("chat_hold_queued", {
      taskId: "edit-save",
      agent: "claude",
      itemId: "q",
    });
    await typeInto(box, "Then run the unit tests");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await released("edit-save", { text: "Then run the unit tests", steer: false });
    const draft = await screen.findByRole("textbox", { name: "Message Claude Code" });
    await waitFor(() => expect(composerText(draft)).toBe("Half a thought"));
    expect(screen.queryByText("Editing")).toBeNull();
  });

  it("steers with the edited text of a queued message", async () => {
    const { box } = await openQueuedEdit("edit-steer");
    await typeInto(box, "Run the tests now");
    fireEvent.click(screen.getByRole("button", { name: "Steer" }));
    await released("edit-steer", { text: "Run the tests now", steer: true });
  });

  it("leaves a queued message unchanged when its edit is cancelled", async () => {
    const { box } = await openQueuedEdit("edit-cancel");
    await typeInto(box, "Never mind");
    fireEvent.keyDown(box, { key: "Escape" });
    await released("edit-cancel", {});
    expect(mocks.invoke).not.toHaveBeenCalledWith("chat_interrupt", expect.anything());
  });

  it("keeps editing when saving fails and shows why", async () => {
    const { box } = await openQueuedEdit("edit-fail");
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_release_queued"
        ? Promise.reject("That message is no longer queued")
        : Promise.resolve(undefined)
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("That message is no longer queued");
    expect(composerText(box)).toBe("Then run the tests");
  });

  it("keeps a queued message held with its edit when the pane closes and reopens", async () => {
    const { view, box } = await openQueuedEdit("edit-close");
    await typeInto(box, "Then run the unit tests");
    view.unmount();
    render(<ChatPane taskId="edit-close" agent="claude" folders={["/wt/a"]} />);
    const reopened = await screen.findByRole("textbox", { name: "Queued message" });
    await waitFor(() => expect(composerText(reopened)).toBe("Then run the unit tests"));
    expect(mocks.invoke).not.toHaveBeenCalledWith("chat_release_queued", expect.anything());
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await released("edit-close", { text: "Then run the unit tests", steer: false });
    const draft = await screen.findByRole("textbox", { name: "Message Claude Code" });
    await waitFor(() => expect(composerText(draft)).toBe("Half a thought"));
  });

  it("only offers to remove queued messages when the agent cannot steer", async () => {
    mocks.catalog = AGENT_CATALOG.map((agent) => ({
      ...agent,
      capabilities: { ...agent.capabilities, steer: false },
    }));
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open" ? Promise.resolve(busyWithQueue()) : Promise.resolve(undefined)
    );
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    await screen.findByRole("button", { name: "Remove queued message" });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Steer" })).toBeNull());
  });

  it("shows why a session failed and restarts it", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open"
        ? Promise.resolve(
            snapshot({ items: [], status: { kind: "failed", message: "Codex is not signed in" } })
          )
        : Promise.resolve(undefined)
    );
    const onStatusChange = vi.fn();
    render(
      <ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} onStatusChange={onStatusChange} />
    );
    await screen.findByText("Codex is not signed in");
    await waitFor(() =>
      expect(onStatusChange).toHaveBeenLastCalledWith({ kind: "exited", code: null })
    );
    expect(screen.getByRole("textbox")).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(screen.getByRole("button", { name: "Restart chat" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "chat_open",
        expect.objectContaining({ mode: "restart" })
      )
    );
  });

  it("detaches its own generation on unmount without closing the session", async () => {
    const view = render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    await screen.findByText("Fix the bug");
    view.unmount();
    expect(mocks.invoke).toHaveBeenCalledWith("chat_detach", {
      taskId: "t1",
      agent: "codex",
      generation: 3,
    });
    expect(mocks.invoke.mock.calls.some(([command]) => command === "chat_close")).toBe(false);
  });

  it("opens the model picker from /model instead of sending it", async () => {
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Claude Code" });
    typeInto(box, "/mo");
    const commands = screen.getByRole("listbox", { name: "Commands" });
    expect(within(commands).getAllByRole("option")[0]).toHaveTextContent("/model");
    fireEvent.keyDown(box, { key: "Enter" });

    const models = screen.getByRole("listbox", { name: "Model" });
    expect(
      within(models)
        .getAllByRole("option")
        .map((o) => o.textContent)
    ).toEqual(["1Opus", "2Haiku"]);
    fireEvent.keyDown(box, { key: "2" });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_configure", {
        taskId: "t1",
        agent: "claude",
        setting: { kind: "model", value: "haiku" },
      })
    );
    expect(mocks.invoke.mock.calls.some(([command]) => command === "chat_send")).toBe(false);
  });

  it("handles typed UI commands and inserts pass-through ones", async () => {
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Claude Code" });
    typeInto(box, "/plan");
    fireEvent.keyDown(box, { key: "Escape" });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_configure", {
        taskId: "t1",
        agent: "claude",
        setting: { kind: "mode", value: "plan" },
      })
    );

    typeInto(box, "/rev");
    fireEvent.keyDown(box, { key: "Tab" });
    expect(composerText(box)).toBe("/review ");
  });

  it("completes @ mentions from the task repositories", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "chat_file_search")
        return Promise.resolve([{ path: "src/chat/ChatPane.tsx", name: "ChatPane.tsx" }]);
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a", "/wt/b"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Codex" });
    typeInto(box, "look at @chatp");
    const option = await screen.findByRole("option", { name: /ChatPane.tsx/ });
    expect(mocks.invoke).toHaveBeenCalledWith("chat_file_search", {
      folders: ["/wt/a", "/wt/b"],
      query: "chatp",
    });
    fireEvent.mouseDown(option);
    expect(composerText(box)).toBe("look at @src/chat/ChatPane.tsx ");
  });

  it("shows the composer controls, plan and context usage", async () => {
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    await screen.findByText("Fix the bug");
    expect(screen.getByRole("button", { name: "Model: Opus" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reasoning effort: high" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Permissions: Ask before edits" }));
    fireEvent.mouseDown(
      within(screen.getByRole("listbox", { name: "Permissions" })).getByText("Plan")
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_configure", {
        taskId: "t1",
        agent: "codex",
        setting: { kind: "mode", value: "plan" },
      })
    );
    expect(screen.getByRole("region", { name: "Plan" })).toHaveTextContent("Plan 1/2");
    expect(
      screen.getByRole("button", { name: "Context 25% used. Show plan usage and limits" })
    ).toBeInTheDocument();
  });

  it("opens /hooks locally and closes it when the session generation changes", async () => {
    const { rerender } = render(<ChatPane taskId="hooks-task" agent="codex" folders={["/wt/a"]} />);
    await screen.findByText("Fix the bug");
    const box = screen.getByRole("textbox", { name: "Message Codex" });
    typeInto(box, "/hooks");
    fireEvent.keyDown(box, { key: "Enter" });
    expect(await screen.findByRole("dialog", { name: "Codex hooks" })).toBeVisible();
    expect(mocks.invoke).toHaveBeenCalledWith("chat_configure", {
      taskId: "hooks-task",
      agent: "codex",
      setting: { kind: "hooks" },
    });
    expect(mocks.invoke.mock.calls.some(([command]) => command === "chat_send")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Close Codex hooks" }));
    typeInto(box, "/hooks");
    fireEvent.keyDown(box, { key: "Escape" });
    fireEvent.keyDown(box, { key: "Enter" });
    expect(await screen.findByRole("dialog", { name: "Codex hooks" })).toBeVisible();
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "chat_open" ? snapshot({ generation: 4 }) : undefined)
    );
    const newControls = {
      ...snapshot().controls,
      hooks: { items: [], loading: false, error: null, warnings: [] },
    };
    emit(3, { type: "controls", controls: newControls });
    expect(screen.getByText("No hooks configured for this worktree.")).toBeVisible();
    rerender(<ChatPane taskId="hooks-task" agent="codex" folders={["/wt/b"]} />);
    await waitFor(() =>
      expect(mocks.invoke.mock.calls.filter(([command]) => command === "chat_open")).toHaveLength(2)
    );
    expect(screen.queryByRole("dialog", { name: "Codex hooks" })).not.toBeInTheDocument();
  });

  it("opens plan usage from /status and the context ring, updating with new limits", async () => {
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    await screen.findByText("Fix the bug");
    const box = screen.getByRole("textbox", { name: "Message Claude Code" });
    typeInto(box, "/status");
    fireEvent.keyDown(box, { key: "Enter" });
    expect(await screen.findByText("Loading usage…")).toBeInTheDocument();
    expect(mocks.invoke).toHaveBeenCalledWith("chat_refresh_usage", {
      taskId: "t1",
      agent: "claude",
    });
    const usage = {
      supported: true,
      plan: "Claude Team",
      account: "a@b.c",
      windows: [
        { id: "five_hour", label: "5-hour limit", usedPercent: 25, resetsAt: 1790809800 },
        { id: "seven_day", label: "Weekly limit", usedPercent: 86, resetsAt: null },
      ],
      session: null,
      error: null,
    };
    emit(3, { type: "controls", controls: { ...snapshot().controls, usage } });
    expect(screen.getByRole("progressbar", { name: "Weekly limit" })).toHaveAttribute(
      "aria-valuenow",
      "86"
    );
    expect(screen.getByText("25% used")).toBeInTheDocument();
    expect(screen.getAllByText(/^Resets/)).toHaveLength(1);
    const appShortcut = vi.fn();
    window.addEventListener("keydown", appShortcut);
    fireEvent.keyDown(document, { key: "Escape" });
    window.removeEventListener("keydown", appShortcut);
    expect(appShortcut).not.toHaveBeenCalled();
    expect(screen.queryByText("Weekly limit")).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Context 25% used. Show plan usage and limits" })
    );
    expect(screen.getByText("Claude Team")).toBeInTheDocument();
  });

  it("shows why plan usage could not be refreshed", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "chat_refresh_usage") return Promise.reject("Claude is not running");
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    await screen.findByText("Fix the bug");
    fireEvent.click(
      screen.getByRole("button", { name: "Context 25% used. Show plan usage and limits" })
    );
    expect(await screen.findByText("Claude is not running")).toBeInTheDocument();
    expect(screen.queryByText("Loading usage…")).not.toBeInTheDocument();
  });

  it.each([
    ["codex", "auto", "Approve for me", "full", "Full access"],
    ["claude", "auto", "Auto", "bypassPermissions", "Bypass permissions"],
  ] as const)(
    "switches %s permissions from automatic to relaxed and manual",
    async (agent, auto, autoLabel, relaxed, relaxedLabel) => {
      const state = snapshot();
      const modes = [
        { id: auto, label: autoLabel, description: "" },
        { id: relaxed, label: relaxedLabel, description: "" },
        { id: "ask", label: "Manual", description: "" },
      ];
      const controls = { ...state.controls, modes, mode: auto };
      mocks.invoke.mockImplementation((command: string) =>
        Promise.resolve(
          command === "chat_open" ? { ...state, controls, status: { kind: "busy" } } : undefined
        )
      );
      render(<ChatPane taskId="t1" agent={agent} folders={["/wt/a"]} />);
      await screen.findByRole("button", { name: `Permissions: ${autoLabel}` });
      for (const mode of [modes[1], modes[2]]) {
        fireEvent.click(screen.getByRole("button", { name: /^Permissions:/ }));
        fireEvent.mouseDown(
          within(screen.getByRole("listbox", { name: "Permissions" })).getByText(mode.label)
        );
        await waitFor(() =>
          expect(mocks.invoke).toHaveBeenCalledWith("chat_configure", {
            taskId: "t1",
            agent,
            setting: { kind: "mode", value: mode.id },
          })
        );
        emit(3, { type: "controls", controls: { ...controls, mode: mode.id } });
        expect(
          screen.getByRole("button", { name: `Permissions: ${mode.label}` })
        ).toBeInTheDocument();
      }
    }
  );

  it("answers the focused approval from the keyboard", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open"
        ? Promise.resolve(
            snapshot({
              status: { kind: "busy" },
              pending: [
                {
                  id: "r1",
                  title: "Run command",
                  detail: "$ ls",
                  format: "text",
                  kind: "approval",
                  decisions: [
                    { id: "accept", label: "Approve" },
                    { id: "acceptForSession", label: "Approve for session" },
                    { id: "decline", label: "Reject" },
                  ],
                },
              ],
            })
          )
        : Promise.resolve(undefined)
    );
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    const card = await screen.findByRole("region", { name: "Run command" });
    await waitFor(() => expect(card).toHaveFocus());
    fireEvent.keyDown(card, { key: "Escape" });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_respond", {
        generation: 3,
        taskId: "t1",
        agent: "codex",
        requestId: "r1",
        response: { decision: "decline" },
      })
    );
  });

  it("summarizes each turn per repository and reviews the conversation's changes", async () => {
    const file = (folder: string, path: string, additions: number, deletions: number) => ({
      folder,
      path,
      status: "modified",
      additions,
      deletions,
      binary: false,
    });
    const turn = {
      before: "b1",
      after: "a1",
      files: [file("/wt/a", "src/x.ts", 1, 1), file("/wt/b", "y.rb", 2, 0)],
    };
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open")
        return Promise.resolve(
          snapshot({
            items: [
              { id: "u", kind: "user", text: "Fix the bug" },
              { id: "a", kind: "assistant", text: "Fixed" },
              { id: "u2", kind: "user", text: "Explain it" },
              { id: "a2", kind: "assistant", text: "Sure" },
            ],
          })
        );
      if (command === "chat_changes")
        return Promise.resolve({
          turns: [
            { items: ["u"], changes: turn, branch: null },
            {
              items: ["u2"],
              changes: { before: "a1", after: "a1", files: [] },
              branch: { before: "base", after: "a1", files: [file("/wt/a", "z.ts", 5, 0)] },
            },
          ],
          conversation: { ...turn, files: [file("/wt/a", "src/x.ts", 3, 1)] },
        });
      if (command === "chat_file_diff")
        return Promise.resolve({
          before: { kind: "text", text: "const a = 1;\n" },
          after: { kind: "text", text: "const a = 2;\n" },
        });
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a", "/wt/b"]} />);

    const card = await screen.findByRole("button", { name: /Review 2 changed files/ });
    expect(card).toHaveTextContent("a · 1 file changed+1 −1");
    expect(card).toHaveTextContent("b · 1 file changed+2 −0");
    expect(screen.getAllByRole("button", { name: /Review/ })).toHaveLength(1);
    expect(screen.getByText("Fixed").compareDocumentPosition(card)).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING
    );

    fireEvent.click(screen.getByRole("button", { name: "Changes +3 −1" }));
    const panel = screen.getByRole("region", { name: "Changes" });
    expect(within(panel).getByLabelText("Changes scope")).toHaveValue("conversation");
    await within(panel).findByText("2", { selector: ".diff-changed" });
    expect(mocks.invoke).toHaveBeenCalledWith("chat_file_diff", {
      taskId: "t1",
      agent: "codex",
      before: "b1",
      after: "a1",
      folder: "/wt/a",
      path: "src/x.ts",
      previousPath: null,
    });

    fireEvent.change(within(panel).getByLabelText("Changes scope"), { target: { value: "0" } });
    const files = within(panel).getByRole("navigation", { name: "Changed files" });
    expect(files).toHaveTextContent("a+1 −1Mx.ts src+1 −1b+2 −0My.rb+2 −0");
    fireEvent.click(within(files).getByRole("button", { name: /y\.rb/ }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "chat_file_diff",
        expect.objectContaining({ folder: "/wt/b", path: "y.rb" })
      )
    );

    fireEvent.click(within(panel).getByRole("button", { name: "Branch" }));
    expect(panel).toHaveTextContent("No file changes.");
    fireEvent.change(within(panel).getByLabelText("Changes scope"), {
      target: { value: "conversation" },
    });
    expect(within(panel).getByRole("navigation", { name: "Changed files" })).toHaveTextContent(
      "Mz.ts+5 −0"
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "chat_file_diff",
        expect.objectContaining({ before: "base", after: "a1", path: "z.ts" })
      )
    );

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("region", { name: "Changes" })).toBeNull();
  });

  it("opens a changed file in the code preview at the chosen line", async () => {
    Element.prototype.scrollIntoView = vi.fn();
    const file = (path: string, status: string) => ({
      folder: "/wt/a",
      path,
      status,
      additions: 1,
      deletions: 1,
      binary: false,
    });
    const set = {
      before: "b1",
      after: "a1",
      files: [file("src/x.ts", "modified"), file("gone.ts", "deleted")],
    };
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open")
        return Promise.resolve(
          snapshot({
            items: [
              { id: "u", kind: "user", text: "Edit" },
              { id: "a", kind: "assistant", text: "Done" },
            ],
          })
        );
      if (command === "chat_changes")
        return Promise.resolve({
          turns: [{ items: ["u"], changes: set, branch: null }],
          conversation: set,
        });
      if (command === "chat_file_diff")
        return Promise.resolve({
          before: { kind: "text", text: "a\nb\n" },
          after: { kind: "text", text: "a\nc\n" },
        });
      if (command === "chat_read_file")
        return Promise.resolve({
          path: "/wt/a/src/x.ts",
          content: { kind: "text", text: "a\nc\n" },
        });
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);

    fireEvent.click(await screen.findByRole("button", { name: /^Changes / }));
    const changes = screen.getByRole("region", { name: "Changes" });
    fireEvent.click(await within(changes).findByRole("button", { name: "Open file at line 2" }));

    const preview = await screen.findByRole("region", { name: "Preview of x.ts" });
    expect(mocks.invoke).toHaveBeenCalledWith("chat_read_file", {
      folders: ["/wt/a"],
      base: null,
      path: "/wt/a/src/x.ts",
    });
    await waitFor(() => expect(preview.querySelector(".diff-selected")).toHaveTextContent("2c"));
    expect(
      within(changes).queryByRole("button", { name: /Open file at line 1$/ })
    ).toBeInTheDocument();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("region", { name: "Preview of x.ts" })).toBeNull();
    expect(screen.getByRole("region", { name: "Changes" })).toBeInTheDocument();

    fireEvent.click(within(changes).getByRole("button", { name: "Open file" }));
    expect(await screen.findByRole("region", { name: "Preview of x.ts" })).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "Escape" });

    fireEvent.click(within(changes).getByRole("button", { name: /gone\.ts/ }));
    await waitFor(() =>
      expect(within(changes).queryByRole("button", { name: /^Open file/ })).toBeNull()
    );
  });

  it("marks files reviewed until a later turn changes them", async () => {
    const file = (path: string, content: string) => ({
      folder: "/wt/a",
      path,
      status: "modified",
      additions: 1,
      deletions: 0,
      binary: false,
      content,
    });
    const first = { before: "b1", after: "a1", files: [file("x.ts", "x1"), file("y.ts", "y1")] };
    const second = { before: "a1", after: "a2", files: [file("x.ts", "x2")] };
    let reviewed = ["/wt/a\ny.ts\ny1"];
    let failNextMark = true;
    mocks.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "chat_open")
        return Promise.resolve(
          snapshot({
            items: [
              { id: "u", kind: "user", text: "Edit" },
              { id: "a", kind: "assistant", text: "Done" },
              { id: "u2", kind: "user", text: "Again" },
              { id: "a2", kind: "assistant", text: "Done again" },
            ],
          })
        );
      if (command === "chat_changes")
        return Promise.resolve({
          turns: [
            { items: ["u"], changes: first, branch: null },
            { items: ["u2"], changes: second, branch: null },
          ],
          conversation: {
            before: "b1",
            after: "a2",
            files: [file("x.ts", "x2"), file("y.ts", "y1")],
          },
        });
      if (command === "chat_reviewed_files") return Promise.resolve(reviewed);
      if (command === "chat_set_file_reviewed") {
        if (failNextMark) {
          failNextMark = false;
          return Promise.reject("disk full");
        }
        reviewed = args!.reviewed
          ? [...reviewed, args!.key as string]
          : reviewed.filter((key) => key !== args!.key);
        return Promise.resolve(reviewed);
      }
      if (command === "chat_file_diff")
        return Promise.resolve({
          before: { kind: "text", text: "a\n" },
          after: { kind: "text", text: "b\n" },
        });
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);

    fireEvent.click(await screen.findByRole("button", { name: /^Changes / }));
    const panel = screen.getByRole("region", { name: "Changes" });
    const files = within(panel).getByRole("navigation", { name: "Changed files" });
    await waitFor(() =>
      expect(within(files).getByRole("button", { name: /y\.ts/ })).toHaveTextContent("Reviewed")
    );
    const checkbox = within(panel).getByRole("checkbox", { name: "Reviewed" });
    expect(checkbox).not.toBeChecked();

    fireEvent.click(checkbox);
    expect(await within(panel).findByRole("alert")).toHaveTextContent("disk full");
    fireEvent.click(checkbox);
    await waitFor(() => expect(checkbox).toBeChecked());
    expect(within(files).getByRole("button", { name: /x\.ts/ })).toHaveTextContent("Reviewed");
    expect(within(panel).queryByRole("alert")).toBeNull();

    fireEvent.change(within(panel).getByLabelText("Changes scope"), { target: { value: "0" } });
    expect(within(files).getByRole("button", { name: /x\.ts/ })).not.toHaveTextContent("Reviewed");
    fireEvent.click(within(panel).getByRole("checkbox", { name: "Reviewed" }));
    await waitFor(() =>
      expect(within(files).getByRole("button", { name: /x\.ts/ })).toHaveTextContent("Reviewed")
    );
    fireEvent.click(within(panel).getByRole("checkbox", { name: "Reviewed" }));
    await waitFor(() =>
      expect(within(files).getByRole("button", { name: /x\.ts/ })).not.toHaveTextContent("Reviewed")
    );
  });

  it("keeps diff comments across panel reopenings and sends them as one message", async () => {
    const turn = {
      before: "b1",
      after: "a1",
      files: [
        {
          folder: "/wt/a",
          path: "x.ts",
          status: "modified",
          additions: 1,
          deletions: 1,
          binary: false,
        },
      ],
    };
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "chat_changes")
        return Promise.resolve({ turns: [{ items: ["u"], changes: turn }], conversation: turn });
      if (command === "chat_file_diff")
        return Promise.resolve({
          before: { kind: "text", text: "const a = 1;\n" },
          after: { kind: "text", text: "const a = 2;\n" },
        });
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);

    const openPanel = async () => {
      fireEvent.click(await screen.findByRole("button", { name: "Changes +1 −1" }));
      return screen.getByRole("region", { name: "Changes" });
    };
    let panel = await openPanel();
    fireEvent.click(await within(panel).findByRole("button", { name: "Comment on line 1" }));
    fireEvent.change(within(panel).getByRole("textbox", { name: "Comment" }), {
      target: { value: "Use a constant" },
    });
    fireEvent.click(within(panel).getByRole("button", { name: "Comment" }));
    fireEvent.keyDown(document, { key: "Escape" });

    panel = await openPanel();
    expect(await within(panel).findByText("Use a constant")).toBeInTheDocument();
    fireEvent.click(within(panel).getByRole("button", { name: "Send 1 comment" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_send", {
        taskId: "t1",
        agent: "codex",
        text: "Review comments on your changes:\n\n/wt/a/x.ts:1\n\n```diff\n+const a = 2;\n```\n\nUse a constant",
        images: [],
      })
    );
    await waitFor(() => expect(screen.queryByRole("region", { name: "Changes" })).toBeNull());
  });
  it("keeps diff comments after remounting and lists those whose diff is gone", async () => {
    const file = {
      folder: "/wt/a",
      path: "x.ts",
      status: "modified",
      additions: 1,
      deletions: 1,
      binary: false,
    };
    let after = "a1";
    mocks.invoke.mockImplementation((command: string) => {
      const turn = { before: "b1", after, files: [file] };
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "chat_changes")
        return Promise.resolve({ turns: [{ items: ["u"], changes: turn }], conversation: turn });
      if (command === "chat_file_diff")
        return Promise.resolve({
          before: { kind: "text", text: "const a = 1;\n" },
          after: { kind: "text", text: "const a = 2;\n" },
        });
      return Promise.resolve(undefined);
    });
    const openPanel = async () => {
      fireEvent.click(await screen.findByRole("button", { name: "Changes +1 −1" }));
      return screen.getByRole("region", { name: "Changes" });
    };
    const first = render(<ChatPane taskId="t-remount" agent="codex" folders={["/wt/a"]} />);
    let panel = await openPanel();
    fireEvent.click(await within(panel).findByRole("button", { name: "Comment on line 1" }));
    fireEvent.change(within(panel).getByRole("textbox", { name: "Comment" }), {
      target: { value: "Use a constant" },
    });
    fireEvent.click(within(panel).getByRole("button", { name: "Comment" }));
    first.unmount();

    after = "a2";
    render(<ChatPane taskId="t-remount" agent="codex" folders={["/wt/a"]} />);
    panel = await openPanel();
    const outdated = within(panel).getByRole("region", { name: "Comments on outdated changes" });
    expect(outdated).toHaveTextContent("/wt/a/x.ts:1 · Use a constant");
    fireEvent.click(within(outdated).getByRole("button", { name: "Discard" }));

    expect(within(panel).queryByRole("button", { name: /Send/ })).toBeNull();
  });
});

describe("Markdown previews", () => {
  const plan =
    "# Ship previews\n\nCards first.\n\n## Steps\n\n1. Build\n2. Test\n3. Ship\n4. Celebrate\n\n## Risks\n\nNone.";

  it("shows a card for referenced Markdown files and opens the rich preview over the chat", async () => {
    mocks.invoke.mockImplementation((command: string, args: { path?: string }) => {
      if (command === "chat_open")
        return Promise.resolve(
          snapshot({
            items: [
              {
                id: "a",
                kind: "assistant",
                text: "Wrote [the plan](docs/plan.md); `missing.md` is next.",
              },
            ],
          })
        );
      if (command === "chat_read_markdown")
        return args.path === "docs/plan.md"
          ? Promise.resolve({ path: "/wt/a/docs/plan.md", text: plan })
          : Promise.reject("File not found: missing.md");
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);

    const card = await screen.findByRole("button", { name: "Preview Ship previews" });
    expect(card).toHaveTextContent("docs/plan.md");
    expect(card).toHaveTextContent("Cards first.Steps1. Build2. Test3. Ship");
    expect(card).not.toHaveTextContent("Celebrate");
    expect(mocks.invoke).toHaveBeenCalledWith("chat_read_markdown", {
      folders: ["/wt/a"],
      path: "missing.md",
    });
    expect(screen.getAllByRole("button", { name: /^Preview / })).toHaveLength(1);

    fireEvent.click(card);
    const panel = await screen.findByRole("region", { name: "Preview of Ship previews" });
    expect(within(panel).getByRole("heading", { level: 2, name: "Steps" })).toBeInTheDocument();
    expect(within(panel).getByRole("navigation", { name: "Outline" })).toHaveTextContent(
      "Ship previewsStepsRisks"
    );

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("region", { name: /Preview of/ })).toBeNull();

    fireEvent.click(screen.getByRole("link", { name: "the plan" }));
    expect(await screen.findByRole("region", { name: "Preview of Ship previews" })).toBeVisible();
  });

  it("opens linked source files in a highlighted preview scrolled to the cited line", async () => {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    mocks.invoke.mockImplementation((command: string, args: { path?: string }) => {
      if (command === "chat_open")
        return Promise.resolve(
          snapshot({
            items: [
              {
                id: "a",
                kind: "assistant",
                text: "See [main.ts:2](src/main.ts:2) and [gone](src/gone.ts).",
              },
            ],
          })
        );
      if (command === "chat_read_file")
        return args.path === "src/main.ts"
          ? Promise.resolve({
              path: "/wt/a/src/main.ts",
              content: { kind: "text", text: "const a = 1;\nexport default a;" },
            })
          : Promise.reject("File not found in the task repositories: src/gone.ts");
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);

    fireEvent.click(await screen.findByRole("link", { name: "src/main.ts:2" }));
    const panel = await screen.findByRole("region", { name: "Preview of main.ts" });
    await waitFor(() => expect(panel).toHaveTextContent("1const a = 1;2export default a;"));
    expect(panel).toHaveTextContent("src/main.ts");
    expect(mocks.invoke).toHaveBeenCalledWith("chat_read_file", {
      folders: ["/wt/a"],
      base: null,
      path: "src/main.ts",
    });
    expect(panel.querySelector(".diff-selected")).toHaveTextContent("2export default a;");
    expect(scrollIntoView).toHaveBeenCalledWith({ block: "center" });

    fireEvent.keyDown(document, { key: "Escape" });
    fireEvent.click(screen.getByRole("link", { name: "gone" }));
    const missing = await screen.findByRole("region", { name: "Preview of gone.ts" });
    expect(await within(missing).findByRole("alert")).toHaveTextContent("File not found");
  });

  it("brings back an unsaved file edit when the chat is shown again", async () => {
    Element.prototype.scrollIntoView = vi.fn();
    let onDisk = "const a = 1;\n";
    mocks.invoke.mockImplementation(
      (command: string, args: { original?: string; text?: string }) => {
        if (command === "chat_open")
          return Promise.resolve(
            snapshot({
              items: [{ id: "a", kind: "assistant", text: "See [main.ts](src/main.ts)." }],
            })
          );
        if (command === "chat_read_file")
          return Promise.resolve({
            path: "/wt/a/src/main.ts",
            content: { kind: "text", text: onDisk },
          });
        if (command === "chat_write_file") {
          onDisk = args.text!;
          return Promise.resolve({
            path: "/wt/a/src/main.ts",
            content: { kind: "text", text: onDisk },
          });
        }
        return Promise.resolve(undefined);
      }
    );
    const pane = (tab?: string) => (
      <ChatPane taskId="kept-edit" agent="claude" tab={tab} folders={["/wt/a"]} />
    );
    const first = render(pane());
    fireEvent.click(await screen.findByRole("link", { name: "src/main.ts" }));
    const panel = await screen.findByRole("region", { name: "Preview of main.ts" });
    fireEvent.click(await within(panel).findByRole("button", { name: "Edit file" }));
    fireEvent.change(within(panel).getByRole("textbox", { name: "File contents" }), {
      target: { value: "const a = 2;\n" },
    });
    first.unmount();

    const otherTab = render(pane("other"));
    await screen.findByRole("link", { name: "src/main.ts" });
    expect(screen.queryByRole("region", { name: /Preview of/ })).toBeNull();
    otherTab.unmount();

    const second = render(pane());
    const kept = await screen.findByRole("region", { name: "Preview of main.ts" });
    expect(within(kept).getByRole("textbox", { name: "File contents" })).toHaveValue(
      "const a = 2;\n"
    );
    fireEvent.click(await within(kept).findByRole("button", { name: "Save" }));
    await waitFor(() => expect(onDisk).toBe("const a = 2;\n"));
    expect(mocks.invoke).toHaveBeenCalledWith(
      "chat_write_file",
      expect.objectContaining({ original: "const a = 1;\n", text: "const a = 2;\n" })
    );
    second.unmount();

    render(pane());
    await screen.findByRole("link", { name: "src/main.ts" });
    expect(screen.queryByRole("region", { name: /Preview of/ })).toBeNull();
  });

  it("forgets an unsaved file edit once it is discarded", async () => {
    Element.prototype.scrollIntoView = vi.fn();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open")
        return Promise.resolve(
          snapshot({
            items: [{ id: "a", kind: "assistant", text: "See [main.ts](src/main.ts)." }],
          })
        );
      if (command === "chat_read_file")
        return Promise.resolve({
          path: "/wt/a/src/main.ts",
          content: { kind: "text", text: "const a = 1;\n" },
        });
      return Promise.resolve(undefined);
    });
    const pane = () => <ChatPane taskId="discarded-edit" agent="claude" folders={["/wt/a"]} />;
    const first = render(pane());
    fireEvent.click(await screen.findByRole("link", { name: "src/main.ts" }));
    const panel = await screen.findByRole("region", { name: "Preview of main.ts" });
    fireEvent.click(await within(panel).findByRole("button", { name: "Edit file" }));
    fireEvent.change(within(panel).getByRole("textbox", { name: "File contents" }), {
      target: { value: "const a = 2;\n" },
    });
    fireEvent.click(within(panel).getByRole("button", { name: "Close preview" }));
    fireEvent.click(within(panel).getByRole("button", { name: "Discard" }));
    first.unmount();

    render(pane());
    await screen.findByRole("link", { name: "src/main.ts" });
    expect(screen.queryByRole("region", { name: /Preview of/ })).toBeNull();
  });

  it("previews a pending plan and approves it from the preview", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open"
        ? Promise.resolve(
            snapshot({
              items: [{ id: "p", kind: "notice", title: "Plan", text: plan }],
              pending: [
                {
                  id: "r1",
                  title: "Claude's plan is ready",
                  detail: plan,
                  format: "markdown",
                  kind: "approval",
                  decisions: [
                    { id: "allow", label: "Approve plan" },
                    { id: "deny", label: "Keep planning" },
                  ],
                },
              ],
            })
          )
        : Promise.resolve(undefined)
    );
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);

    const cards = await screen.findAllByRole("button", { name: "Preview Ship previews" });
    expect(cards).toHaveLength(1);
    fireEvent.click(cards[0]);
    const panel = screen.getByRole("region", { name: "Preview of Ship previews" });
    fireEvent.click(within(panel).getByRole("button", { name: "Approve plan" }));

    await waitFor(() => expect(screen.queryByRole("region", { name: /Preview of/ })).toBeNull());
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_respond", {
        generation: 3,
        taskId: "t1",
        agent: "claude",
        requestId: "r1",
        response: { decision: "allow" },
      })
    );
  });

  const pendingPlan = () =>
    snapshot({
      status: { kind: "busy" },
      items: [{ id: "p", kind: "notice", title: "Plan", text: plan }],
      pending: [
        {
          id: "r1",
          title: "Claude's plan is ready",
          detail: plan,
          format: "markdown",
          kind: "approval",
          feedback: true,
          editablePlan: true,
          decisions: [
            { id: "allow", label: "Approve plan" },
            { id: "deny", label: "Keep planning" },
          ],
        },
      ],
    });

  const openPendingPlan = async () => {
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a", "/wt/b"]} />);
    fireEvent.click(await screen.findByRole("button", { name: "Preview Ship previews" }));
    return screen.getByRole("region", { name: "Preview of Ship previews" });
  };

  it("keeps edited markdown after closing the preview and approves it from the card", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "chat_open" ? pendingPlan() : undefined)
    );
    const panel = await openPendingPlan();
    fireEvent.click(within(panel).getByRole("button", { name: "Edit plan" }));
    const edited = "# Revised plan\n\n1. Test first";
    fireEvent.change(within(panel).getByRole("textbox", { name: "Plan markdown" }), {
      target: { value: edited },
    });
    fireEvent.click(within(panel).getByRole("button", { name: "Preview plan" }));
    expect(within(panel).getByRole("heading", { name: "Revised plan", level: 1 })).toBeVisible();
    fireEvent.click(within(panel).getByRole("button", { name: "Close preview" }));
    fireEvent.click(screen.getByRole("button", { name: "Preview Revised plan" }));
    expect(screen.getByRole("region", { name: "Preview of Revised plan" })).toHaveTextContent(
      "Test first"
    );
    fireEvent.keyDown(document, { key: "Escape" });
    fireEvent.keyDown(screen.getByRole("region", { name: "Claude's plan is ready" }), {
      key: "Enter",
    });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_respond", {
        generation: 3,
        taskId: "t1",
        agent: "claude",
        requestId: "r1",
        response: { decision: "allow", plan: edited },
      })
    );
  });

  it("rejects a plan with feedback without approving the edited input", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "chat_open" ? pendingPlan() : undefined)
    );
    const panel = await openPendingPlan();
    fireEvent.change(
      within(panel).getByRole("textbox", { name: "Rejection feedback (optional)" }),
      { target: { value: "Include rollback steps" } }
    );
    fireEvent.click(within(panel).getByRole("button", { name: "Keep planning" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_respond", {
        generation: 3,
        taskId: "t1",
        agent: "claude",
        requestId: "r1",
        response: { decision: "deny", message: "Include rollback steps" },
      })
    );
  });

  it("blocks approving an empty plan while allowing feedback", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "chat_open" ? pendingPlan() : undefined)
    );
    const panel = await openPendingPlan();
    fireEvent.click(within(panel).getByRole("button", { name: "Edit plan" }));
    fireEvent.change(within(panel).getByRole("textbox", { name: "Plan markdown" }), {
      target: { value: " " },
    });
    expect(within(panel).getByRole("button", { name: "Approve plan" })).toBeDisabled();
    expect(
      within(panel).getByRole("button", { name: "Approve in new conversation" })
    ).toBeDisabled();
    expect(within(panel).getByRole("button", { name: "Keep planning" })).toBeEnabled();
  });

  it("opens a fresh conversation with the edited plan as its first message", async () => {
    mocks.invoke.mockImplementation((command: string, args: { mode?: string }) => {
      if (command === "chat_open")
        return Promise.resolve(
          args.mode === "fresh" ? snapshot({ generation: 4, items: [] }) : pendingPlan()
        );
      return Promise.resolve(undefined);
    });
    const panel = await openPendingPlan();
    fireEvent.click(within(panel).getByRole("button", { name: "Edit plan" }));
    fireEvent.change(within(panel).getByRole("textbox", { name: "Plan markdown" }), {
      target: { value: "# Approved\n\nTest first" },
    });
    fireEvent.click(within(panel).getByRole("button", { name: "Approve in new conversation" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_send", {
        taskId: "t1",
        agent: "claude",
        text: "Implement the following approved plan:\n\n# Approved\n\nTest first",
        images: [],
      })
    );
    expect(mocks.invoke).toHaveBeenCalledWith(
      "chat_open",
      expect.objectContaining({
        mode: "fresh",
        folders: ["/wt/a", "/wt/b"],
        conversation: undefined,
        handoffFrom: undefined,
      })
    );
    expect(mocks.invoke).not.toHaveBeenCalledWith("chat_respond", expect.anything());
    expect(screen.queryByRole("region", { name: /Preview of/ })).toBeNull();
  });

  it("keeps the preview and feedback available when responding fails", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_open") return Promise.resolve(pendingPlan());
      if (command === "chat_respond") return Promise.reject("Could not send response");
      return Promise.resolve(undefined);
    });
    const panel = await openPendingPlan();
    fireEvent.change(
      within(panel).getByRole("textbox", { name: "Rejection feedback (optional)" }),
      { target: { value: "Include rollback steps" } }
    );
    fireEvent.click(within(panel).getByRole("button", { name: "Keep planning" }));
    expect(await within(panel).findByRole("alert")).toHaveTextContent("Could not send response");
    expect(
      within(panel).getByRole("textbox", { name: "Rejection feedback (optional)" })
    ).toHaveValue("Include rollback steps");
    expect(within(panel).getByRole("button", { name: "Keep planning" })).toBeEnabled();
  });

  it("retains the approved markdown and shows an error if the first message fails", async () => {
    mocks.invoke.mockImplementation((command: string, args: { mode?: string }) => {
      if (command === "chat_open")
        return Promise.resolve(
          args.mode === "fresh" ? snapshot({ generation: 4, items: [] }) : pendingPlan()
        );
      if (command === "chat_send") return Promise.reject("Could not send approved plan");
      return Promise.resolve(undefined);
    });
    const panel = await openPendingPlan();
    fireEvent.click(within(panel).getByRole("button", { name: "Edit plan" }));
    fireEvent.change(within(panel).getByRole("textbox", { name: "Plan markdown" }), {
      target: { value: "# Revised\n\nPreserve the data" },
    });
    fireEvent.click(within(panel).getByRole("button", { name: "Approve in new conversation" }));
    const retained = await screen.findByRole("region", { name: "Preview of Revised" });
    expect(await within(retained).findByRole("alert")).toHaveTextContent(
      "Could not send approved plan"
    );
    expect(within(retained).getByText("Preserve the data")).toBeVisible();
    expect(mocks.invoke).not.toHaveBeenCalledWith("chat_respond", expect.anything());
  });

  it("does not send the plan after leaving the pane during a fresh launch", async () => {
    let finishOpen: (value: ReturnType<typeof snapshot>) => void = () => undefined;
    mocks.invoke.mockImplementation((command: string, args: { mode?: string }) => {
      if (command === "chat_open")
        return args.mode === "fresh"
          ? new Promise((resolve) => {
              finishOpen = resolve;
            })
          : Promise.resolve(pendingPlan());
      return Promise.resolve(undefined);
    });
    const view = render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    fireEvent.click(await screen.findByRole("button", { name: "Approve in new conversation" }));
    view.unmount();
    await act(async () => finishOpen(snapshot({ generation: 4, items: [] })));
    expect(mocks.invoke).not.toHaveBeenCalledWith("chat_send", expect.anything());
  });

  it("does not send the plan if opening the new conversation fails", async () => {
    mocks.invoke.mockImplementation((command: string, args: { mode?: string }) => {
      if (command === "chat_open")
        return args.mode === "fresh"
          ? Promise.reject("Could not launch Claude")
          : Promise.resolve(pendingPlan());
      return Promise.resolve(undefined);
    });
    const panel = await openPendingPlan();
    fireEvent.click(within(panel).getByRole("button", { name: "Approve in new conversation" }));
    expect(await screen.findByRole("region", { name: "Preview of Ship previews" })).toBeVisible();
    await waitFor(() =>
      expect(
        within(screen.getByRole("region", { name: "Preview of Ship previews" })).getByRole("alert")
      ).toHaveTextContent("Could not launch Claude")
    );
    expect(mocks.invoke).not.toHaveBeenCalledWith("chat_send", expect.anything());
    expect(mocks.invoke).not.toHaveBeenCalledWith("chat_respond", expect.anything());
  });
});

it.each(["claude", "codex"] as const)(
  "shows usage for the current chat agent: %s",
  async (agent) => {
    render(<ChatPane taskId="t1" agent={agent} folders={["/wt/a"]} />);
    await screen.findByRole("textbox", {
      name: `Message ${describeAgent(agent).label}`,
    });
    const indicator = screen.queryByRole("button", { name: "Claude Code usage and limits" });
    if (agent === "claude") {
      expect(indicator).toBeInTheDocument();
      expect(mocks.invoke).toHaveBeenCalledWith("agent_cached_usage", { agent: "claude" });
    } else {
      expect(screen.getByRole("button", { name: "Codex usage and limits" })).toBeInTheDocument();
      expect(mocks.invoke).toHaveBeenCalledWith("agent_cached_usage", { agent: "codex" });
      expect(indicator).not.toBeInTheDocument();
      expect(mocks.invoke).not.toHaveBeenCalledWith("agent_cached_usage", { agent: "claude" });
    }
  }
);

describe("switching agents", () => {
  it("offers only installed agents", async () => {
    mocks.catalog = AGENT_CATALOG.map((agent) => ({ ...agent, installed: agent.id === "claude" }));
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} onSwitchAgent={vi.fn()} />);
    await screen.findByText("Fix the bug");
    fireEvent.click(screen.getByRole("button", { name: "Agent: Claude Code" }));
    await waitFor(() =>
      expect(
        within(screen.getByRole("listbox", { name: "Agent" })).getAllByRole("option")
      ).toHaveLength(1)
    );
  });

  it("runs an agent outside the catalog under its id and offers the installed ones", async () => {
    render(<ChatPane taskId="t1" agent="gemini" folders={["/wt/a"]} onSwitchAgent={vi.fn()} />);
    await screen.findByText("Fix the bug");
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "Message gemini" })).toHaveAttribute(
        "aria-placeholder",
        expect.stringMatching(/^Message gemini — /)
      )
    );
    fireEvent.click(screen.getByRole("button", { name: "Agent: gemini" }));
    expect(
      within(screen.getByRole("listbox", { name: "Agent" }))
        .getAllByRole("option")
        .map((option) => option.textContent)
    ).toEqual(["1Claude Code", "2Codex", "3gemini"]);
  });

  const pickAgent = async (label: string) => {
    fireEvent.click(await screen.findByRole("button", { name: /^Agent: / }));
    fireEvent.mouseDown(within(screen.getByRole("listbox", { name: "Agent" })).getByText(label));
  };

  it("asks before handing a conversation in progress to the other agent", async () => {
    const onSwitchAgent = vi.fn();
    render(
      <ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} onSwitchAgent={onSwitchAgent} />
    );
    await screen.findByText("Fix the bug");
    expect(screen.getByRole("button", { name: "Agent: Claude Code" })).toHaveClass(
      "text-agent-claude"
    );
    await pickAgent("Codex");
    const dialog = screen.getByRole("dialog", { name: "Switch to Codex?" });
    expect(dialog).toHaveTextContent("reasoning, raw tool output");
    fireEvent.click(within(dialog).getByRole("button", { name: "Continue in Codex" }));
    expect(onSwitchAgent).toHaveBeenLastCalledWith({
      agent: "codex",
      mode: "fresh",
      handoffFrom: "claude",
    });

    await pickAgent("Codex");
    fireEvent.click(screen.getByRole("button", { name: "New conversation" }));
    expect(onSwitchAgent).toHaveBeenLastCalledWith({
      agent: "codex",
      mode: "fresh",
      handoffFrom: undefined,
    });
  });

  it("switches an empty chat directly and refuses while the agent works", async () => {
    const onSwitchAgent = vi.fn();
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "chat_open" ? snapshot({ items: [] }) : undefined)
    );
    render(
      <ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} onSwitchAgent={onSwitchAgent} />
    );
    await pickAgent("Claude Code");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onSwitchAgent).toHaveBeenCalledWith({ agent: "claude", mode: "fresh" });

    onSwitchAgent.mockClear();
    emit(3, { type: "status", status: { kind: "busy" } });
    await pickAgent("Claude Code");
    expect(onSwitchAgent).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("Wait for Codex to finish");
  });

  it("opens as the remounting switch asked, once", async () => {
    const takeOpenIntent = vi
      .fn()
      .mockReturnValueOnce({ agent: "codex", mode: "fresh", handoffFrom: "claude" })
      .mockReturnValue(null);
    render(
      <ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} takeOpenIntent={takeOpenIntent} />
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "chat_open",
        expect.objectContaining({ agent: "codex", mode: "fresh", handoffFrom: "claude" })
      )
    );
  });

  it("shows carried messages read-only after the switch divider", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "chat_open"
          ? snapshot({
              items: [
                { id: "carried:s:u", kind: "user", text: "Earlier ask", origin: "claude" },
                {
                  id: "handoff:s",
                  kind: "handoff",
                  text: "Continued in Codex",
                  detail: "Earlier messages are from Claude Code.",
                },
                { id: "u", kind: "user", text: "Now this", checkpoint: "c1" },
              ],
            })
          : undefined
      )
    );
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    const carried = await screen.findByText("Earlier ask");
    expect(carried.closest("[data-origin]")).toHaveAttribute("data-origin", "claude");
    expect(screen.getByRole("separator", { name: "Continued in Codex" })).toHaveTextContent(
      "Earlier messages are from Claude Code."
    );
    expect(screen.getAllByRole("button", { name: "Rewind or fork" })).toHaveLength(1);
  });
});

describe("Find in conversation", () => {
  const message = (index: number) => ({
    id: `m${index}`,
    kind: "assistant",
    text: `Message ${index}`,
  });
  const range = (from: number, to: number) =>
    Array.from({ length: to - from }, (_, index) => message(from + index));
  const openFind = async () => {
    const composer = await screen.findByRole("textbox");
    act(() => composer.focus());
    fireEvent.keyDown(composer, { key: "f", metaKey: true });
    return screen.findByRole("textbox", { name: "Find in conversation" });
  };
  const status = () => within(screen.getByRole("search")).getByRole("status");

  beforeEach(() => {
    Object.defineProperty(Range.prototype, "getBoundingClientRect", {
      configurable: true,
      value(this: Range) {
        return this.startContainer
          .parentElement!.closest("[data-row-key]")!
          .getBoundingClientRect();
      },
    });
  });
  afterEach(() => {
    delete (Range.prototype as Partial<Range>).getBoundingClientRect;
  });

  it("counts matches from the newest one and moves through them with Enter", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "chat_open"
          ? snapshot({
              items: [
                { id: "u", kind: "user", text: "Fix the BUG" },
                { id: "c", kind: "command", text: "grep bug" },
                { id: "a", kind: "assistant", text: "The bug is fixed; no bug left" },
              ],
            })
          : undefined
      )
    );
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    const input = await openFind();
    expect(input).toHaveFocus();
    fireEvent.change(input, { target: { value: "bug" } });
    expect(status()).toHaveTextContent("3/3");
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(status()).toHaveTextContent("3/3");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(status()).toHaveTextContent("2/3");
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(status()).toHaveTextContent("1/3");
    fireEvent.click(screen.getByRole("button", { name: "Previous match" }));
    expect(status()).toHaveTextContent("3/3");

    fireEvent.change(input, { target: { value: "missing" } });
    expect(status()).toHaveTextContent("No results");
    expect(screen.getByRole("button", { name: "Next match" })).toBeDisabled();

    fireEvent.keyDown(input, { key: "Escape" });
    expect(screen.queryByRole("search")).not.toBeInTheDocument();
    expect(screen.getByRole("textbox")).toHaveFocus();
  });

  it("focuses the open find bar again instead of searching the workspace", async () => {
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    const input = await openFind();
    fireEvent.change(input, { target: { value: "care" } });
    const composer = screen.getAllByRole("textbox").find((box) => box !== input)!;
    act(() => composer.focus());
    fireEvent.keyDown(composer, { key: "f", metaKey: true });
    expect(input).toHaveFocus();
    expect(input).toHaveValue("care");
  });

  it("highlights the rendered matches and the current one until the bar closes", async () => {
    const highlights = new Map<string, Range[]>();
    vi.stubGlobal("CSS", { highlights });
    vi.stubGlobal(
      "Highlight",
      class {
        constructor(...ranges: Range[]) {
          return ranges;
        }
      }
    );
    const texts = (name: string) => highlights.get(name)?.map((range) => range.toString());
    try {
      render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
      const input = await openFind();
      fireEvent.change(input, { target: { value: "the" } });
      expect(texts("chat-find")).toEqual(["the"]);
      expect(texts("chat-find-current")).toEqual(["the"]);
      fireEvent.change(input, { target: { value: "" } });
      expect(texts("chat-find")).toEqual([]);
      expect(highlights.has("chat-find-current")).toBe(false);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("highlights only the message text, not the titles and controls around it", async () => {
    const highlights = new Map<string, Range[]>();
    vi.stubGlobal("CSS", { highlights });
    vi.stubGlobal(
      "Highlight",
      class {
        constructor(...ranges: Range[]) {
          return ranges;
        }
      }
    );
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "chat_open"
          ? snapshot({
              items: [
                { id: "n", kind: "notice", title: "Bug report", text: "bug found" },
                { id: "e", kind: "error", text: "bug crashed" },
              ],
            })
          : undefined
      )
    );
    try {
      render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
      const input = await openFind();
      fireEvent.change(input, { target: { value: "bug" } });
      expect(status()).toHaveTextContent("2/2");
      const [current] = highlights.get("chat-find-current")!;
      expect(current.startContainer.textContent).toBe("bug crashed");
      expect(highlights.get("chat-find")?.map((range) => range.startContainer.textContent)).toEqual(
        ["bug found", "bug crashed"]
      );
      fireEvent.keyDown(input, { key: "Escape" });
      expect(highlights.get("chat-find")).toEqual([]);
      expect(highlights.has("chat-find-current")).toBe(false);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("loads the earlier history while searching", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "chat_open"
          ? snapshot({ sessionId: "s1", items: range(10, 20), earlier: 10 })
          : undefined
      )
    );
    const loads = () =>
      mocks.invoke.mock.calls.filter(([command]) => command === "chat_load_earlier");
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    const input = await openFind();
    expect(loads()).toEqual([]);
    fireEvent.change(input, { target: { value: "Message 1" } });
    expect(loads()).toHaveLength(1);
    expect(status()).toHaveTextContent("10/10…");

    emit(3, { type: "earlier", items: range(5, 10), earlier: 5 });
    expect(loads()).toHaveLength(2);
    emit(3, { type: "earlier", items: range(0, 5), earlier: 0 });
    expect(status()).toHaveTextContent("11/11");
    expect(loads()).toHaveLength(2);
  });

  it("scrolls to a match that is not rendered yet", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "chat_open" ? snapshot({ items: range(0, 120) }) : undefined)
    );
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    await screen.findByText("Message 119");
    const scroller = screen.getByText("Message 119").closest(".chat-scroll") as HTMLElement;
    layOutScroller(scroller, 600, () => 120);
    resize();
    expect(screen.queryByText("Message 5")).not.toBeInTheDocument();

    const input = await openFind();
    fireEvent.change(input, { target: { value: "message 5" } });
    expect(status()).toHaveTextContent("11/11");
    expect(screen.getByText("Message 59")).toBeInTheDocument();
    expect(scroller.scrollTop).toBe(59 * 120 - 200);

    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(status()).toHaveTextContent("1/11");
    expect(screen.getByText("Message 5")).toBeInTheDocument();
  });
});

describe("Conversation minimap", () => {
  const turn = (index: number) => [
    { id: `u${index}`, kind: "user", text: `Prompt  ${index}\nin detail` },
    ...Array.from({ length: 8 }, (_, step) => ({
      id: `a${index}.${step}`,
      kind: "assistant",
      text: `Reply ${index}.${step}`,
    })),
  ];
  const conversation = (turns: number) =>
    Array.from({ length: turns }, (_, index) => turn(index)).flat();
  const openWith = (items: unknown[]) => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(command === "chat_open" ? snapshot({ items }) : undefined)
    );
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    return screen.findByRole("navigation", { name: "Conversation messages" });
  };
  const mark = (position: number) =>
    screen.getByRole("button", { name: `Jump to message ${position}` });

  it("stays hidden until the conversation has two messages", async () => {
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    await screen.findByText("care");
    expect(
      screen.queryByRole("navigation", { name: "Conversation messages" })
    ).not.toBeInTheDocument();
  });

  it("previews a message and its last reply on hover or focus", async () => {
    const nav = await openWith([...turn(0), { id: "u1", kind: "user", text: "Follow up" }]);
    fireEvent.mouseEnter(mark(1));
    const tooltip = screen.getByRole("tooltip");
    expect(tooltip).toHaveTextContent(/^Prompt 0 in detailReply 0\.7$/);
    expect(mark(1)).toHaveAttribute("aria-describedby", tooltip.id);
    fireEvent.mouseLeave(nav);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();

    act(() => mark(2).focus());
    expect(screen.getByRole("tooltip")).toHaveTextContent(/^Follow up$/);
    expect(fireEvent.keyDown(mark(2), { key: "Escape" })).toBe(false);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    expect(fireEvent.keyDown(mark(2), { key: "Escape" })).toBe(true);

    act(() => mark(1).focus());
    expect(screen.getByRole("tooltip")).toBeInTheDocument();
    act(() => mark(1).blur());
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("moves between the marks with the arrow keys, Home and End", async () => {
    const nav = await openWith(conversation(4));
    expect(mark(4)).toHaveAttribute("aria-current", "location");
    expect(mark(4)).toHaveAttribute("tabindex", "0");
    expect(mark(1)).toHaveAttribute("tabindex", "-1");

    act(() => mark(4).focus());
    fireEvent.keyDown(mark(4), { key: "ArrowUp" });
    expect(mark(3)).toHaveFocus();
    fireEvent.keyDown(mark(3), { key: "Home" });
    fireEvent.keyDown(mark(1), { key: "ArrowUp" });
    expect(mark(1)).toHaveFocus();
    fireEvent.keyDown(mark(1), { key: "End" });
    fireEvent.keyDown(mark(4), { key: "ArrowDown" });
    fireEvent.keyDown(mark(4), { key: "a" });
    expect(mark(4)).toHaveFocus();

    act(() => mark(4).blur());
    fireEvent.keyDown(nav, { key: "ArrowUp" });
    expect(document.body).toHaveFocus();
  });

  it("jumps to a message that is not rendered and marks the message in view", async () => {
    const nav = await openWith(conversation(12));
    const scroller = nav.closest(".chat-scroll") as HTMLElement;
    layOutScroller(scroller, 600, () => 120);
    resize();
    expect(screen.queryByText("Prompt 2 in detail")).not.toBeInTheDocument();

    fireEvent.click(mark(3));
    expect(screen.getByText("Prompt 2 in detail")).toBeInTheDocument();
    expect(scroller.scrollTop).toBe(18 * 120 - 16);
    fireEvent.scroll(scroller);
    expect(mark(3)).toHaveAttribute("aria-current", "location");

    scroller.scrollTop = scroller.scrollHeight;
    fireEvent.scroll(scroller);
    expect(screen.queryByText("Prompt 2 in detail")).not.toBeInTheDocument();
    expect(mark(12)).toHaveAttribute("aria-current", "location");
  });

  it("keeps the current find match mounted after a jump", async () => {
    Object.defineProperty(Range.prototype, "getBoundingClientRect", {
      configurable: true,
      value: () => ({ top: 0, bottom: 0 }),
    });
    try {
      const nav = await openWith(conversation(12));
      const scroller = nav.closest(".chat-scroll") as HTMLElement;
      layOutScroller(scroller, 600, () => 120);
      resize();
      const composer = screen.getByRole("textbox");
      act(() => composer.focus());
      fireEvent.keyDown(composer, { key: "f", metaKey: true });
      const input = await screen.findByRole("textbox", { name: "Find in conversation" });
      fireEvent.change(input, { target: { value: "Reply 1.3" } });
      expect(screen.getByText("Reply 1.3")).toBeInTheDocument();

      fireEvent.click(mark(12));
      fireEvent.scroll(scroller);
      expect(mark(12)).toHaveAttribute("aria-current", "location");
      expect(screen.getByText("Reply 1.3")).toBeInTheDocument();
    } finally {
      delete (Range.prototype as Partial<Range>).getBoundingClientRect;
    }
  });
});
