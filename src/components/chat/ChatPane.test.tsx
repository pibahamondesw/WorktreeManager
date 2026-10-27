// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { EditorView } from "@codemirror/view";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  channels: [] as { onmessage?: (m: unknown) => void }[],
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
  Channel: class {
    onmessage?: (m: unknown) => void;
    constructor() {
      mocks.channels.push(this);
    }
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }));

import { ChatPane } from "./ChatPane";
import { AGENTS } from "../../services/agents";

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
  act(() => mocks.channels.at(-1)!.onmessage?.({ generation, event }));

beforeEach(() => {
  mocks.channels.length = 0;
  mocks.invoke.mockReset().mockImplementation((command: string) => {
    if (command === "chat_open") return Promise.resolve(snapshot());
    if (command === "check_app_installed") return Promise.resolve(true);
    return Promise.resolve(undefined);
  });
});
afterEach(cleanup);

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
    fireEvent.click(screen.getByRole("button", { name: "Worked · 1 step" }));
    fireEvent.click(screen.getByRole("button", { name: /Edited 1 file/ }));
    expect(screen.getByText("+added")).toHaveClass("text-success");
  });

  it("renders the latest messages and loads earlier ones on scroll or request", async () => {
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
    expect(screen.getByText("Message 70")).toBeInTheDocument();
    expect(screen.queryByText("Message 69")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Show earlier messages (70)" }));
    expect(screen.getByText("Message 20")).toBeInTheDocument();
    expect(screen.queryByText("Message 19")).not.toBeInTheDocument();

    const scroller = screen.getByText("Message 119").closest(".chat-scroll") as HTMLElement;
    const itemHeight = 100;
    Object.defineProperties(scroller, {
      scrollHeight: {
        configurable: true,
        get: () => scroller.querySelectorAll(".chat-markdown").length * itemHeight,
      },
      clientHeight: { configurable: true, value: 500 },
      scrollTop: { configurable: true, writable: true, value: 0 },
    });
    const scrollTo = (scrollTop: number) => {
      scroller.scrollTop = scrollTop;
      fireEvent.scroll(scroller);
    };

    scrollTo(scroller.scrollHeight - 500);
    emit(3, { type: "upsert", item: { id: "m120", kind: "assistant", text: "Message 120" } });
    expect(screen.queryByText("Message 20")).not.toBeInTheDocument();
    expect(screen.getByText("Message 71")).toBeInTheDocument();

    scrollTo(3000);
    emit(3, { type: "upsert", item: { id: "m121", kind: "assistant", text: "Message 121" } });
    expect(screen.getByText("Message 71")).toBeInTheDocument();
    expect(screen.getByText("Message 121")).toBeInTheDocument();

    scrollTo(100);
    expect(screen.getByText("Message 21")).toBeInTheDocument();
    expect(scroller.scrollTop).toBe(100 + 50 * itemHeight);

    scrollTo(100);
    expect(screen.getByText("Message 0")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Show earlier messages/ })).not.toBeInTheDocument();

    scrollTo(scroller.scrollHeight);
    expect(screen.queryByText("Message 71")).not.toBeInTheDocument();
    expect(screen.getByText("Message 72")).toBeInTheDocument();
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
    expect(screen.queryByRole("button", { name: /Working/ })).not.toBeInTheDocument();

    const first = screen.getByRole("region", { name: "Subagent: Find the bug" });
    fireEvent.click(within(first).getAllByRole("button")[0]);
    expect(within(first).getByText("Sub reply one")).toBeInTheDocument();
    expect(within(first).queryByText("Sub reply two")).not.toBeInTheDocument();
    expect(
      within(screen.getByRole("region", { name: "Subagent: Read the docs" })).getByText("1 step")
    ).toBeInTheDocument();
  });

  it("collapses each run of work items into one expandable message", async () => {
    const work = (n: number) => {
      if (n === 17) return [{ id: `r${n}`, kind: "reasoning", text: "" }];
      const status = n === 18 ? "inProgress" : n === 19 ? "failed" : "completed";
      return [
        { id: `r${n}`, kind: "reasoning", text: `Thinking ${n}` },
        { id: `c${n}`, kind: "command", text: `ls ${n}`, status },
      ];
    };
    const turn = (n: number) => [
      { id: `u${n}`, kind: "user", text: `Prompt ${n}` },
      ...work(n),
      { id: `a${n}`, kind: "assistant", text: `Answer ${n}` },
    ];
    const items = Array.from({ length: 20 }, (_, n) => turn(n)).flat();
    items.push(
      { id: "thinking", kind: "reasoning", text: "" },
      { id: "live", kind: "tool", text: "Read file", status: "inProgress" }
    );
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "chat_open" ? snapshot({ status: { kind: "busy" }, items }) : undefined
      )
    );
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} />);
    await screen.findByText("Answer 19");
    expect(screen.getByText("Answer 3")).toBeInTheDocument();
    expect(screen.queryByText("Prompt 3")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Show earlier messages (11)" })).toBeInTheDocument();
    expect(screen.queryByText("Thinking 19")).not.toBeInTheDocument();
    expect(screen.queryByText("ls 19")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Working · 1 step" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Worked · 2 steps" })).toHaveLength(14);
    expect(screen.queryByRole("button", { name: /Worked · 1 step/ })).not.toBeInTheDocument();

    const failedWork = screen.getByRole("button", { name: "Worked · 2 steps · 1 failed" });
    fireEvent.click(failedWork);
    expect(failedWork).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Thinking 19")).toBeInTheDocument();
    expect(screen.getByText("ls 19")).toBeInTheDocument();
    expect(screen.queryByText("Thinking 18")).not.toBeInTheDocument();
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

  it("sends on Enter, streams the reply and ignores a stale generation", async () => {
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Claude" });
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
    const box = await screen.findByRole("textbox", { name: "Message Claude" });
    const png = (name: string) => new File(["hi"], name, { type: "image/png" });
    fireEvent.paste(box, { clipboardData: { files: [png("one.png")] } });
    fireEvent.drop(box.parentElement!, {
      dataTransfer: { types: ["Files"], files: [png("two.png"), new File(["x"], "a.txt")] },
    });
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

  it("skips unsupported image types and reports oversized ones", async () => {
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Claude" });
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
      clipboardData: { files: [new File(["hi"], "one.png", { type: "image/png" })] },
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByRole("list", { name: "Attached images" })).not.toBeInTheDocument();
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
    const box = screen.getByRole("textbox", { name: "Message Claude" });
    expect(box).toHaveAttribute("aria-disabled", "true");
    expect(box).toHaveAttribute("aria-placeholder", "Starting Claude…");

    await act(async () => resolveOpen(snapshot()));
    expect(box).not.toHaveAttribute("aria-disabled");
    expect(box).toHaveFocus();
  });

  it.each([
    ["claude", "Claude is working — messages you send now run after this turn (Esc to stop)"],
    ["codex", "Codex is working — messages you send now are added to this turn (Esc to stop)"],
  ] as const)("tells how %s handles messages sent mid-turn", async (agent, placeholder) => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_open"
        ? Promise.resolve(snapshot({ status: { kind: "busy" } }))
        : Promise.resolve(undefined)
    );
    render(<ChatPane taskId="t1" agent={agent} folders={["/wt/a"]} />);
    const box = screen.getByRole("textbox", { name: `Message ${AGENTS[agent].label}` });
    await waitFor(() => expect(box).toHaveAttribute("aria-placeholder", placeholder));
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
    const box = await screen.findByRole("textbox", { name: "Message Claude" });
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
    const box = await screen.findByRole("textbox", { name: "Message Claude" });
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
    const box = screen.getByRole("textbox", { name: "Message Claude" });
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
        taskId: "t1",
        agent: "codex",
        requestId: "r1",
        response: { decision: "decline" },
      })
    );
  });

  it("lists the worktree's conversations, resumes one and renames another", async () => {
    mocks.invoke.mockImplementation((command: string, args?: { agent?: string }) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "check_app_installed") return Promise.resolve(true);
      if (command === "chat_sessions" && args?.agent === "codex")
        return Promise.reject("codex app-server crashed");
      if (command === "chat_sessions")
        return Promise.resolve({
          sessions: [
            { id: "c1", title: "Current work", updatedAt: 20, current: true },
            { id: "c0", title: "Older work", updatedAt: 10, current: false },
          ],
          canRename: true,
          canArchive: false,
        });
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    await screen.findByText("Fix the bug");
    fireEvent.click(screen.getByRole("button", { name: "Conversations" }));
    const list = await screen.findByRole("list", { name: "Conversations" });
    expect(within(list).queryByRole("button", { name: "Archive" })).toBeNull();
    expect(within(list).getByRole("button", { name: /Current work/ })).toBeDisabled();
    expect(screen.getByText("Codex: codex app-server crashed")).toBeInTheDocument();

    fireEvent.click(within(list).getAllByRole("button", { name: "Rename" })[1]);
    const name = screen.getByLabelText("Conversation name");
    fireEvent.change(name, { target: { value: "Renamed" } });
    fireEvent.submit(name.closest("form")!);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_session_rename", {
        taskId: "t1",
        agent: "claude",
        folders: ["/wt/a"],
        id: "c0",
        title: "Renamed",
      })
    );

    fireEvent.click(await screen.findByRole("button", { name: /Older work/ }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "chat_open",
        expect.objectContaining({ mode: "restart", conversation: "c0" })
      )
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
            { items: ["u"], changes: turn },
            { items: ["u2"], changes: { before: "a1", after: "a1", files: [] } },
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

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("region", { name: "Changes" })).toBeNull();
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

    expect(screen.queryByRole("region", { name: /Preview of/ })).toBeNull();
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_respond", {
        taskId: "t1",
        agent: "claude",
        requestId: "r1",
        response: { decision: "allow" },
      })
    );
  });
});

it.each(["claude", "codex"] as const)(
  "shows usage for the current chat agent: %s",
  async (agent) => {
    render(<ChatPane taskId="t1" agent={agent} folders={["/wt/a"]} />);
    await screen.findByRole("textbox", {
      name: `Message ${agent === "claude" ? "Claude" : "Codex"}`,
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
  it("offers only installed agents and lists only their conversations", async () => {
    mocks.invoke.mockImplementation((command: string, args?: { agent?: string }) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "check_app_installed") return Promise.resolve(false);
      if (command === "chat_sessions")
        return args?.agent === "claude"
          ? Promise.resolve({ sessions: [], canRename: false, canArchive: false })
          : Promise.reject("codex CLI not found");
      return Promise.resolve(undefined);
    });
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} onSwitchAgent={vi.fn()} />);
    await screen.findByText("Fix the bug");
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("check_app_installed", { editor: "codex" })
    );
    fireEvent.click(screen.getByRole("button", { name: "Agent: Claude" }));
    expect(
      within(screen.getByRole("listbox", { name: "Agent" })).getAllByRole("option")
    ).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Conversations" }));
    expect(await screen.findByText("No conversations in this worktree yet.")).toBeInTheDocument();
    expect(mocks.invoke).not.toHaveBeenCalledWith(
      "chat_sessions",
      expect.objectContaining({ agent: "codex" })
    );
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
    expect(screen.getByRole("button", { name: "Agent: Claude" })).toHaveClass("text-agent-claude");
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
      Promise.resolve(
        command === "chat_open"
          ? snapshot({ items: [] })
          : command === "check_app_installed" || undefined
      )
    );
    render(
      <ChatPane taskId="t1" agent="codex" folders={["/wt/a"]} onSwitchAgent={onSwitchAgent} />
    );
    await pickAgent("Claude");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onSwitchAgent).toHaveBeenCalledWith({ agent: "claude", mode: "fresh" });

    onSwitchAgent.mockClear();
    emit(3, { type: "status", status: { kind: "busy" } });
    await pickAgent("Claude");
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

  it("resumes another agent's conversation from the shared list", async () => {
    const onSwitchAgent = vi.fn();
    mocks.invoke.mockImplementation((command: string, args?: { agent?: string }) => {
      if (command === "chat_open") return Promise.resolve(snapshot());
      if (command === "check_app_installed") return Promise.resolve(true);
      if (command === "chat_sessions")
        return Promise.resolve({
          sessions:
            args?.agent === "codex"
              ? [{ id: "th1", title: "Codex thread", updatedAt: 30, current: true }]
              : [{ id: "c1", title: "Claude session", updatedAt: 20, current: true }],
          canRename: false,
          canArchive: false,
        });
      return Promise.resolve(undefined);
    });
    render(
      <ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} onSwitchAgent={onSwitchAgent} />
    );
    await screen.findByText("Fix the bug");
    fireEvent.click(screen.getByRole("button", { name: "Conversations" }));
    const list = await screen.findByRole("list", { name: "Conversations" });
    const rows = within(list).getAllByRole("button");
    expect(rows.map((row) => row.textContent)).toEqual([
      expect.stringContaining("Codex thread"),
      expect.stringContaining("Claude session"),
    ]);
    expect(rows[0]).toBeEnabled();
    expect(rows[1]).toBeDisabled();
    fireEvent.click(rows[0]);
    expect(onSwitchAgent).toHaveBeenCalledWith({
      agent: "codex",
      mode: "restart",
      conversation: "th1",
    });
  });
});
