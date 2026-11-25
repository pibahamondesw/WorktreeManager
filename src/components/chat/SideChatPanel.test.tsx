// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import type { InvokeArgs } from "@tauri-apps/api/core";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args?: InvokeArgs) => Promise<unknown>>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: InvokeArgs) =>
    invokeWithAgentCatalog(mocks.invoke, command, args),
  Channel: class {
    onmessage?: (m: unknown) => void;
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn().mockResolvedValue(() => undefined) }),
}));
vi.mock("../../observeResize", () => import("../../test/fakeObserveResize"));

import { SideChatPanel } from "./SideChatPanel";
import { FirstPromptContext } from "./firstPromptContext";
import { invokeWithAgentCatalog } from "../../services/agents.fixture";

const chat = (items: unknown[], status = "idle") => ({
  generation: 1,
  status: { kind: status },
  items,
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
});

const opening = (snapshot: unknown) =>
  mocks.invoke.mockImplementation((command: string) =>
    Promise.resolve(command === "chat_open" ? snapshot : undefined)
  );

const renderPanel = (onBringBack = vi.fn()) => {
  const firstPrompt = vi.fn().mockResolvedValue("The task's first prompt");
  render(
    <FirstPromptContext.Provider value={firstPrompt}>
      <SideChatPanel
        taskId="t1"
        agent="claude"
        tab="side-1"
        folders={["/wt/a"]}
        takeOpenIntent={() => null}
        onBringBack={onBringBack}
        onClose={vi.fn()}
      />
    </FirstPromptContext.Provider>
  );
  return { onBringBack, firstPrompt };
};

beforeEach(() => mocks.invoke.mockReset());
afterEach(cleanup);

it("starts empty, without the task's first prompt or a reply to add", async () => {
  opening(chat([]));
  const { firstPrompt } = renderPanel();
  expect(
    await screen.findByText("Ask about the conversation. Nothing here is added to it.")
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Add reply to main chat" })).toBeDisabled();
  expect(firstPrompt).not.toHaveBeenCalled();
});

it("adds its latest reply to the main chat and offers no rewind or fork", async () => {
  opening(
    chat([
      { id: "q1", kind: "user", text: "What is it?" },
      { id: "r1", kind: "assistant", text: "A parser." },
      { id: "q2", kind: "user", text: "And this?" },
      { id: "r2", kind: "assistant", text: "Its tests." },
      { id: "t", kind: "tool", text: "", title: "Read" },
    ])
  );
  const { onBringBack } = renderPanel();
  await screen.findByText("Its tests.");
  expect(screen.queryByRole("button", { name: "Rewind or fork" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Add reply to main chat" }));
  expect(onBringBack).toHaveBeenCalledWith("Its tests.");
});

it("waits for the reply to finish before adding it", async () => {
  opening(chat([{ id: "r", kind: "assistant", text: "Half of" }], "busy"));
  renderPanel();
  await screen.findByText("Half of");
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Add reply to main chat" })).toBeDisabled()
  );
});

it("says an ended side chat cannot start over instead of offering a restart", async () => {
  mocks.invoke.mockImplementation((command: string) =>
    command === "chat_open"
      ? Promise.reject("This side chat has ended. Ask again with /btw.")
      : Promise.resolve(undefined)
  );
  renderPanel();
  expect(
    await screen.findByText("This side chat has ended. Ask again with /btw.")
  ).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Restart chat" })).toBeNull();
});
