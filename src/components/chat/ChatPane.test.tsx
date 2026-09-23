// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

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
  ...overrides,
});

const emit = (generation: number, event: unknown) =>
  act(() => mocks.channels.at(-1)!.onmessage?.({ generation, event }));

beforeEach(() => {
  mocks.channels.length = 0;
  mocks.invoke.mockReset().mockImplementation((command: string) => {
    if (command === "chat_open") return Promise.resolve(snapshot());
    return Promise.resolve(undefined);
  });
});
afterEach(cleanup);

describe("ChatPane", () => {
  it("restores the transcript and expands file diffs", async () => {
    render(<ChatPane taskId="t1" agent="codex" folders={["/wt/a", "/wt/b"]} />);
    await screen.findByText("Fix the bug");
    expect(mocks.invoke).toHaveBeenCalledWith(
      "chat_open",
      expect.objectContaining({
        taskId: "t1",
        agent: "codex",
        folders: ["/wt/a", "/wt/b"],
        restart: false,
      })
    );
    expect(screen.getByText("care").tagName).toBe("STRONG");
    fireEvent.click(screen.getByRole("button", { name: /Edited 1 file/ }));
    expect(screen.getByText("+added")).toHaveClass("text-success");
  });

  it("sends on Enter, streams the reply and ignores a stale generation", async () => {
    render(<ChatPane taskId="t1" agent="claude" folders={["/wt/a"]} />);
    const box = await screen.findByRole("textbox", { name: "Message Claude" });
    fireEvent.change(box, { target: { value: "Add tests" } });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_send", {
        taskId: "t1",
        agent: "claude",
        text: "Add tests",
      })
    );
    expect(box).toHaveValue("");

    emit(3, { type: "status", status: { kind: "busy" } });
    emit(3, { type: "upsert", item: { id: "b", kind: "assistant", text: "Work" } });
    emit(3, { type: "delta", id: "b", field: "text", delta: "ing" });
    emit(2, { type: "delta", id: "b", field: "text", delta: " STALE" });
    expect(screen.getByText("Working")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(mocks.invoke).toHaveBeenCalledWith("chat_interrupt", { taskId: "t1", agent: "claude" });
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
                  kind: "approval",
                  decisions: [
                    { id: "accept", label: "Approve" },
                    { id: "decline", label: "Reject" },
                  ],
                },
                {
                  id: "q1",
                  title: "Codex has a question",
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
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
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
    expect(onStatusChange).toHaveBeenLastCalledWith({ kind: "exited", code: null });
    expect(screen.getByRole("textbox")).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Restart chat" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "chat_open",
        expect.objectContaining({ restart: true })
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
});
