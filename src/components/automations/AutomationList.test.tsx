// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { AutomationList } from "./AutomationList";
import { AutomationSummary } from "../../services/automations";

afterEach(cleanup);

const summary = (name: string, overrides: Partial<AutomationSummary> = {}): AutomationSummary => ({
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
  ...overrides,
});

const issue = { path: "/x", field: "steps", kind: "missing", message: "is required" };
const AUTOMATIONS = [
  summary("babysitter"),
  summary("nightly", {
    state: { enabled: false, pause_reason: "new", next_run_at: null, consecutive_failures: 0 },
  }),
  summary("broken", {
    entry: { ...summary("broken").entry, definition: null, errors: [issue, issue] },
  }),
];

it("lists automations with their status and selects one", () => {
  const onSelect = vi.fn();
  render(
    <AutomationList
      automations={AUTOMATIONS}
      error={null}
      selectedName="nightly"
      onSelect={onSelect}
    />
  );
  expect(screen.getByRole("button", { name: /babysitter/ })).toHaveTextContent("Active");
  expect(screen.getByRole("button", { name: /nightly/ })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("button", { name: /nightly/ })).toHaveTextContent("Paused");
  expect(screen.getByRole("button", { name: /broken/ })).toHaveTextContent("Invalid · 2 errors");
  fireEvent.click(screen.getByRole("button", { name: /broken/ }));
  expect(onSelect).toHaveBeenCalledWith("broken");
});

it("moves focus between automations with the arrow keys", () => {
  render(
    <AutomationList automations={AUTOMATIONS} error={null} selectedName={null} onSelect={vi.fn()} />
  );
  const [first, second, third] = screen.getAllByRole("button");
  first.focus();
  fireEvent.keyDown(first, { key: "ArrowDown" });
  expect(second).toHaveFocus();
  fireEvent.keyDown(second, { key: "ArrowDown" });
  fireEvent.keyDown(third, { key: "ArrowDown" });
  expect(third).toHaveFocus();
  fireEvent.keyDown(third, { key: "ArrowUp" });
  expect(second).toHaveFocus();
  fireEvent.keyDown(second, { key: "Enter" });
  expect(second).toHaveFocus();
});

it("shows the empty state and loading errors", () => {
  const { rerender } = render(
    <AutomationList automations={[]} error={null} selectedName={null} onSelect={vi.fn()} />
  );
  expect(screen.getByText("No automations yet")).toBeInTheDocument();
  rerender(
    <AutomationList
      automations={[]}
      error="Could not read /automations: denied"
      selectedName={null}
      onSelect={vi.fn()}
    />
  );
  expect(screen.getByText("Could not read /automations: denied")).toHaveClass("select-text");
  expect(screen.queryByText("No automations yet")).not.toBeInTheDocument();
});
