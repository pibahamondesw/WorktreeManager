// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AutomationsModal } from "./AutomationsModal";
import { AutomationSummary } from "../../services/automations";
import { InboxItem } from "../../services/automationInbox";
import { useEditorOcclusion } from "../../hooks/useEditorOcclusion";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion: vi.fn() }));

const summary = (name: string): AutomationSummary => ({
  entry: {
    name,
    path: `/automations/${name}/AUTOMATION.md`,
    definition: { name, description: null, trigger: { manual: true, schedule: null }, steps: [] },
    errors: [],
    warnings: [],
  },
  state: { enabled: true, pause_reason: null, next_run_at: null, consecutive_failures: 0 },
  stateError: null,
  latestRun: null,
});
const AUTOMATIONS = [summary("babysitter"), summary("nightly")];

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(invoke).mockReset().mockResolvedValue({ runs: [], next_before: null });
});

const ENTRY: InboxItem = {
  key: "run:babysitter:1",
  automation: "babysitter",
  runId: "1",
  kind: "findings",
  title: "Findings to review",
  detail: "2 findings: PR #12 is red",
  at: 0,
  test: false,
  target: { taskId: "t1", agent: "claude", conversationId: "c1" },
  autoArchived: false,
  notify: true,
  read: false,
  archived: false,
};

const openConversation = vi.fn(() => true);
const setRead = vi.fn();

function Harness({
  automations = AUTOMATIONS,
  error = null as string | null,
  items = [] as InboxItem[],
}) {
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [inboxSelected, setInboxSelected] = useState(false);
  return (
    <>
      <button onClick={() => setOpen(true)}>Open automations</button>
      <AutomationsModal
        open={open}
        onClose={() => setOpen(false)}
        automations={automations}
        error={error}
        onChanged={() => {}}
        selectedName={selected}
        onSelect={(name) => {
          setSelected(name);
          setInboxSelected(false);
        }}
        editorApp="cursor"
        inbox={{ items, unread: items.length, setRead, setArchived: vi.fn() }}
        inboxSelected={inboxSelected}
        onSelectInbox={() => setInboxSelected(true)}
        onOpenConversation={openConversation}
      />
    </>
  );
}

it("closes with Escape when focus is not inside the dialog", () => {
  render(<Harness />);
  const opener = screen.getByRole("button", { name: "Open automations" });
  opener.focus();
  fireEvent.click(opener);
  expect(screen.getByRole("dialog", { name: "Automations" })).toBeInTheDocument();
  expect(useEditorOcclusion).toHaveBeenLastCalledWith(true);
  document.body.focus();
  fireEvent.keyDown(document.body, { key: "Escape" });
  expect(screen.queryByRole("dialog", { name: "Automations" })).not.toBeInTheDocument();
  expect(opener).toHaveFocus();
});

it("closes from its close button", () => {
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "Open automations" }));
  fireEvent.click(screen.getByRole("button", { name: "Close Automations" }));
  expect(screen.queryByRole("dialog", { name: "Automations" })).not.toBeInTheDocument();
});

it("shows the first automation's detail, focused, and switches with the list", () => {
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "Open automations" }));
  const first = screen.getByRole("tab", { name: /babysitter/ });
  expect(first).toHaveFocus();
  expect(screen.getByRole("heading", { name: "babysitter" })).toBeInTheDocument();
  fireEvent.keyDown(first, { key: "ArrowDown" });
  expect(screen.getByRole("heading", { name: "nightly" })).toBeInTheDocument();
  expect(screen.getByRole("tabpanel", { name: /nightly/ })).toBeInTheDocument();
});

it("opens an Inbox entry's conversation from the Inbox tab", () => {
  render(<Harness items={[ENTRY]} />);
  fireEvent.click(screen.getByRole("button", { name: "Open automations" }));
  const inbox = screen.getByRole("tab", { name: /Inbox/ });
  expect(inbox).toHaveTextContent("1 unread");
  fireEvent.keyDown(screen.getByRole("tab", { name: /babysitter/ }), { key: "ArrowUp" });
  expect(inbox).toHaveFocus();
  expect(screen.getByRole("tabpanel", { name: "Inbox" })).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Unread: Findings to review" }));

  expect(setRead).toHaveBeenCalledWith([ENTRY.key], true);
  expect(openConversation).toHaveBeenCalledWith(ENTRY.target);
});

it("shows the empty state and selectable load errors", () => {
  const { rerender } = render(<Harness automations={[]} />);
  fireEvent.click(screen.getByRole("button", { name: "Open automations" }));
  expect(screen.getByText("No automations yet")).toBeInTheDocument();
  rerender(<Harness automations={[]} error="Could not read /automations: denied" />);
  expect(screen.getByText("Could not read /automations: denied")).toHaveClass("select-text");
  expect(screen.queryByText("No automations yet")).not.toBeInTheDocument();
});
