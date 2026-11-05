// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { AutomationInbox } from "./AutomationInbox";
import { InboxItem } from "../../services/automationInbox";

afterEach(cleanup);

const item = (key: string, overrides: Partial<InboxItem> = {}): InboxItem => ({
  key,
  automation: "babysitter",
  runId: "1",
  kind: "findings",
  title: `Entry ${key}`,
  detail: null,
  at: 0,
  test: false,
  target: { taskId: "t1", agent: "claude", conversationId: "c1" },
  autoArchived: false,
  notify: true,
  read: false,
  archived: false,
  ...overrides,
});

const ITEMS = [
  item("a", { detail: "2 findings", test: true }),
  item("b", { kind: "question", read: true, target: null }),
  item("c", { kind: "noop", autoArchived: true, archived: true }),
  item("d", { kind: "failed", archived: true }),
];

function renderInbox(items = ITEMS, onOpen = vi.fn(() => true)) {
  const onSetRead = vi.fn();
  const onSetArchived = vi.fn();
  render(
    <AutomationInbox
      items={items}
      onOpen={onOpen}
      onSetRead={onSetRead}
      onSetArchived={onSetArchived}
    />
  );
  return { onOpen, onSetRead, onSetArchived };
}

const entry = (name: RegExp) => screen.getByRole("button", { name });

it("lists the entries that are not archived, unread first in weight", () => {
  renderInbox();
  const list = screen.getByRole("list", { name: "Inbox entries" });
  expect(within(list).getAllByRole("listitem")).toHaveLength(2);
  expect(entry(/^Unread: Entry a/)).toHaveTextContent("2 findings");
  expect(entry(/^Unread: Entry a/)).toHaveTextContent("babysitter · ");
  expect(entry(/^Unread: Entry a/)).toHaveTextContent("test run");
  expect(entry(/^Entry b/)).toHaveTextContent("no task");
  expect(screen.getByText("1 unread")).toBeInTheDocument();
});

it("moves with the arrows and archives, reads and opens with the keyboard", () => {
  const { onOpen, onSetRead, onSetArchived } = renderInbox();
  const first = entry(/^Unread: Entry a/);
  first.focus();
  fireEvent.keyDown(first, { key: "ArrowDown" });
  expect(entry(/^Entry b/)).toHaveFocus();
  fireEvent.keyDown(entry(/^Entry b/), { key: "ArrowDown" });
  expect(entry(/^Entry b/)).toHaveFocus();
  fireEvent.keyDown(entry(/^Entry b/), { key: "Home" });
  expect(first).toHaveFocus();
  fireEvent.keyDown(first, { key: "End" });
  expect(entry(/^Entry b/)).toHaveFocus();
  fireEvent.keyDown(entry(/^Entry b/), { key: "ArrowUp" });
  expect(first).toHaveFocus();
  fireEvent.keyDown(first, { key: "x" });

  fireEvent.keyDown(first, { key: "e" });
  expect(onSetArchived).toHaveBeenCalledWith(["a"], true);
  fireEvent.keyDown(first, { key: "U" });
  expect(onSetRead).toHaveBeenCalledWith(["a"], true);
  fireEvent.keyDown(entry(/^Entry b/), { key: "u" });
  expect(onSetRead).toHaveBeenCalledWith(["b"], false);

  fireEvent.click(first);
  expect(onOpen).toHaveBeenCalledWith(ITEMS[0]);
  expect(onSetRead).toHaveBeenLastCalledWith(["a"], true);
  fireEvent.click(entry(/^Entry b/));
  expect(onOpen).toHaveBeenCalledTimes(1);
});

it("marks, archives and shows the archived entries from the buttons", () => {
  const { onSetRead, onSetArchived } = renderInbox();
  fireEvent.click(screen.getByRole("button", { name: "Mark all read" }));
  expect(onSetRead).toHaveBeenCalledWith(["a", "c", "d"], true);
  fireEvent.click(screen.getAllByRole("button", { name: "Mark read" })[0]);
  expect(onSetRead).toHaveBeenLastCalledWith(["a"], true);
  fireEvent.click(screen.getByRole("button", { name: "Mark unread" }));
  expect(onSetRead).toHaveBeenLastCalledWith(["b"], false);
  fireEvent.click(screen.getAllByRole("button", { name: "Archive" })[0]);
  expect(onSetArchived).toHaveBeenCalledWith(["a"], true);

  fireEvent.click(screen.getByRole("button", { name: "Archived" }));
  expect(screen.getByRole("button", { name: "Archived" })).toHaveAttribute("aria-pressed", "true");
  const noop = entry(/^Unread: Entry c/);
  fireEvent.keyDown(noop, { key: "e" });
  expect(onSetArchived).toHaveBeenCalledTimes(1);
  expect(screen.getAllByRole("button", { name: "Unarchive" })).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "Unarchive" }));
  expect(onSetArchived).toHaveBeenLastCalledWith(["d"], false);
});

it("says when an entry's task is gone and when there is nothing to show", () => {
  renderInbox(
    [item("a")],
    vi.fn(() => false)
  );
  fireEvent.click(entry(/^Unread: Entry a/));
  expect(screen.getByRole("alert")).toHaveTextContent("The task of this entry no longer exists");
  fireEvent.click(screen.getByRole("button", { name: "Archived" }));
  expect(screen.getByText("No archived entries")).toBeInTheDocument();
  cleanup();
  renderInbox([]);
  expect(screen.getByText("Nothing needs you")).toBeInTheDocument();
  expect(screen.getByText("All read")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Mark all read" })).toBeDisabled();
});

it("approves with edited params or rejects with a comment from the entry", async () => {
  const approval = item("e", {
    kind: "approval",
    title: "Ship it?",
    approval: {
      runId: "7",
      item: "routine",
      step: "ship",
      attempt: 1,
      params: { files: ["a.rb"] },
      timeoutAt: 5000,
      default: "approve",
      answer: null,
    },
  });
  const onAnswer = vi.fn(() => Promise.resolve());
  render(
    <AutomationInbox
      items={[
        approval,
        item("f", {
          ...approval,
          key: "f",
          approval: { ...approval.approval!, answer: "Approved" },
        }),
      ]}
      onOpen={vi.fn(() => true)}
      onSetRead={vi.fn()}
      onSetArchived={vi.fn()}
      onAnswer={onAnswer}
    />
  );
  const form = screen.getByRole("group", { name: "Answer Ship it?" });
  expect(screen.getAllByRole("group")).toHaveLength(1);
  expect(within(form).getByText(/Approves on its own at/)).toBeInTheDocument();
  const files = within(form).getByLabelText("files");
  fireEvent.change(files, { target: { value: "b.rb" } });
  fireEvent.click(within(form).getByRole("button", { name: "Approve" }));
  expect(within(form).getByRole("alert")).toHaveTextContent('files must be JSON, like ["a.rb"]');
  expect(onAnswer).not.toHaveBeenCalled();

  fireEvent.change(files, { target: { value: '["b.rb"]' } });
  fireEvent.click(within(form).getByRole("button", { name: "Approve" }));
  expect(onAnswer).toHaveBeenLastCalledWith(approval, {
    decision: "approve",
    params: { files: ["b.rb"] },
  });
  await screen.findByRole("button", { name: "Reject" });

  onAnswer.mockImplementationOnce(() => Promise.reject("No item of run 7 is waiting on step ship"));
  fireEvent.change(within(form).getByLabelText("Comment"), { target: { value: " Split it " } });
  fireEvent.click(within(form).getByRole("button", { name: "Reject" }));
  expect(onAnswer).toHaveBeenLastCalledWith(approval, { decision: "reject", comment: "Split it" });
  expect(await within(form).findByRole("alert")).toHaveTextContent(
    "No item of run 7 is waiting on step ship"
  );
});

it("shows a removed task up front, with nothing to open and no error", () => {
  const onOpen = vi.fn(() => true);
  const onSetRead = vi.fn();
  render(
    <AutomationInbox
      items={[
        item("a"),
        item("b", { target: { taskId: "t2", agent: null, conversationId: null } }),
      ]}
      onOpen={onOpen}
      onSetRead={onSetRead}
      onSetArchived={vi.fn()}
      taskIds={new Set(["t2"])}
    />
  );
  const removed = entry(/^Unread: Entry a/);
  expect(within(removed).getByText("Task removed")).toHaveAttribute(
    "title",
    "The entry's task and its worktrees were deleted, so there is nothing left to open."
  );
  expect(within(entry(/^Unread: Entry b/)).queryByText("Task removed")).not.toBeInTheDocument();
  fireEvent.click(removed);
  expect(onSetRead).toHaveBeenCalledWith(["a"], true);
  expect(onOpen).not.toHaveBeenCalled();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("focuses the requested entry once the Inbox shows", async () => {
  render(
    <AutomationInbox
      items={[item("a"), item("b")]}
      onOpen={vi.fn(() => true)}
      onSetRead={vi.fn()}
      onSetArchived={vi.fn()}
      focusKey="b"
    />
  );
  await waitFor(() => expect(entry(/^Unread: Entry b/)).toHaveFocus());
  expect(entry(/^Unread: Entry b/)).toHaveAttribute("tabindex", "0");
});
