// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { ChatInfo, ChatItem, ChatStatus } from "../../services/chat";
import { ChatItemView } from "./ChatItemView";
import { ItemDetailContext } from "./itemDetailContext";
import { TaskConversationsContext } from "./taskConversationContext";
import { useTabChat } from "../../hooks/useTabChat";

vi.mock("../../hooks/useTabChat", () => ({ useTabChat: vi.fn(() => null) }));
afterEach(cleanup);

const launch = { taskId: "t2", agent: "codex" as const, tab: "a1", conversationId: "th1" };

const command = (extra: Partial<ChatItem> = {}): ChatItem => ({
  id: "c1",
  kind: "command",
  text: "",
  title: "wtm task chat t2 --agent codex --subagent --message @brief.md",
  status: "completed",
  subagentLaunch: launch,
  ...extra,
});

const chat = (status: ChatStatus, waiting = false): ChatInfo => ({
  sessionId: "s2",
  taskId: "t2",
  agent: "codex",
  tab: "a1",
  origin: "user",
  status,
  waiting,
  backgroundAgents: false,
});

function renderCard(item: ChatItem, expand = vi.fn()) {
  const open = vi.fn(() => true);
  const label = (taskId: string) => (taskId === "t2" ? "WOR-2" : null);
  render(
    <TaskConversationsContext.Provider value={{ label, open }}>
      <ItemDetailContext.Provider value={expand}>
        <ChatItemView item={item} />
      </ItemDetailContext.Provider>
    </TaskConversationsContext.Provider>
  );
  return open;
}

it("shows a subagent launch as a subagent that opens its task's conversation", () => {
  const open = renderCard(command());
  const card = screen.getByRole("region", { name: "Subagent: WOR-2" });
  expect(useTabChat).toHaveBeenCalledWith("t2", "codex", "a1");
  expect(card.querySelector("svg.agent-logo-openai")).not.toBeNull();
  fireEvent.click(within(card).getByRole("button", { name: "Open conversation" }));
  expect(open).toHaveBeenCalledWith(launch);
});

it("names a launch as asked and keeps its task beside the name", () => {
  renderCard(command({ subagentLaunch: { ...launch, name: "Review" } }));
  expect(screen.getByRole("region", { name: "Subagent: Review" })).toHaveTextContent(
    "Subagent · Review · WOR-2"
  );
});

it.each([
  [chat({ kind: "busy" }), "running"],
  [chat({ kind: "starting" }), "running"],
  [chat({ kind: "idle" }, true), "waiting"],
  [chat({ kind: "idle" }), "done"],
])("shows how the launched chat is doing", (info, label) => {
  vi.mocked(useTabChat).mockReturnValueOnce(info);
  renderCard(command());
  expect(screen.getByRole("region", { name: "Subagent: WOR-2" })).toHaveTextContent(label);
});

it("shows no status once the launched chat ended", () => {
  vi.mocked(useTabChat).mockReturnValueOnce(chat({ kind: "exited", code: 0 }));
  renderCard(command());
  const card = screen.getByRole("region", { name: "Subagent: WOR-2" });
  for (const label of ["running", "waiting", "done"]) expect(card).not.toHaveTextContent(label);
});

it("expands to the command and the reply the launch waited for", () => {
  const expand = vi.fn();
  const response = { ok: true, data: { subagent: true, reply: "Shipped the PR." } };
  renderCard(command({ detail: `noise\n{broken\n${JSON.stringify(response)}` }), expand);
  fireEvent.click(screen.getByRole("button", { name: /Subagent · WOR-2/ }));
  expect(expand).toHaveBeenCalledWith("c1", "full");
  expect(screen.getByText(/--subagent --message @brief.md/)).toBeInTheDocument();
  expect(screen.getByText("Shipped the PR.")).toBeInTheDocument();
});

it("shows a structured reply as JSON and no reply when the launch did not wait", () => {
  const structured = { ok: true, data: { subagent: true, reply: { approved: true } } };
  renderCard(command({ detail: JSON.stringify(structured) }));
  fireEvent.click(screen.getByRole("button", { name: /Subagent · WOR-2/ }));
  expect(screen.getByRole("region", { name: "Subagent: WOR-2" })).toHaveTextContent(
    '"approved": true'
  );
  cleanup();
  renderCard(command({ detail: JSON.stringify({ ok: true, data: { subagent: true } }) }));
  fireEvent.click(screen.getByRole("button", { name: /Subagent · WOR-2/ }));
  expect(screen.queryByText(/approved/)).toBeNull();
});

it("offers no way to a launch whose task is gone", () => {
  renderCard(command({ subagentLaunch: { ...launch, taskId: "gone" } }));
  const card = screen.getByRole("region", { name: "Subagent" });
  expect(card).not.toHaveTextContent("Subagent ·");
  expect(within(card).queryByRole("button", { name: "Open conversation" })).toBeNull();
});

it("shows other commands as commands", () => {
  renderCard(command({ subagentLaunch: undefined }));
  expect(screen.queryByRole("region", { name: /Subagent/ })).toBeNull();
});
