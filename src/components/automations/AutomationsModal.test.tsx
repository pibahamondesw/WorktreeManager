// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AutomationsModal } from "./AutomationsModal";
import { AutomationSummary } from "../../services/automations";
import { useEditorOcclusion } from "../../hooks/useEditorOcclusion";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion: vi.fn() }));

const summary = (name: string): AutomationSummary => ({
  entry: {
    name,
    path: `/automations/${name}/AUTOMATION.md`,
    definition: { name, description: null, trigger: { manual: true, schedule: null } },
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

function Harness({ automations = AUTOMATIONS, error = null as string | null }) {
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  return (
    <>
      <button onClick={() => setOpen(true)}>Open automations</button>
      <AutomationsModal
        open={open}
        onClose={() => setOpen(false)}
        automations={automations}
        error={error}
        selectedName={selected}
        onSelect={setSelected}
        editorApp="cursor"
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

it("shows the empty state and selectable load errors", () => {
  const { rerender } = render(<Harness automations={[]} />);
  fireEvent.click(screen.getByRole("button", { name: "Open automations" }));
  expect(screen.getByText("No automations yet")).toBeInTheDocument();
  rerender(<Harness automations={[]} error="Could not read /automations: denied" />);
  expect(screen.getByText("Could not read /automations: denied")).toHaveClass("select-text");
  expect(screen.queryByText("No automations yet")).not.toBeInTheDocument();
});
