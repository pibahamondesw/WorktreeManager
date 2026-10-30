// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { useState } from "react";
import { AutomationList } from "./AutomationList";
import { AutomationSummary } from "../../services/automations";

afterEach(cleanup);

const summary = (name: string, overrides: Partial<AutomationSummary> = {}): AutomationSummary => ({
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

const tab = (name: string) => screen.getByRole("tab", { name: new RegExp(name) });

it("lists automations with their status and selects one", () => {
  const onSelect = vi.fn();
  render(<AutomationList automations={AUTOMATIONS} selectedName="nightly" onSelect={onSelect} />);
  expect(tab("babysitter")).toHaveTextContent("Active");
  expect(tab("nightly")).toHaveAttribute("aria-selected", "true");
  expect(tab("nightly")).toHaveTextContent("Paused");
  expect(tab("broken")).toHaveTextContent("Invalid · 2 errors");
  fireEvent.click(tab("broken"));
  expect(onSelect).toHaveBeenCalledWith("broken");
});

it("selects and focuses automations with the arrow, Home and End keys", () => {
  function Harness() {
    const [selected, setSelected] = useState("babysitter");
    return (
      <AutomationList automations={AUTOMATIONS} selectedName={selected} onSelect={setSelected} />
    );
  }
  render(<Harness />);
  tab("babysitter").focus();
  fireEvent.keyDown(tab("babysitter"), { key: "ArrowDown" });
  expect(tab("nightly")).toHaveFocus();
  expect(tab("nightly")).toHaveAttribute("aria-selected", "true");
  fireEvent.keyDown(tab("nightly"), { key: "End" });
  expect(tab("broken")).toHaveFocus();
  fireEvent.keyDown(tab("broken"), { key: "ArrowDown" });
  expect(tab("babysitter")).toHaveFocus();
  fireEvent.keyDown(tab("babysitter"), { key: "ArrowUp" });
  expect(tab("broken")).toHaveFocus();
  fireEvent.keyDown(tab("broken"), { key: "Home" });
  expect(tab("babysitter")).toHaveFocus();
  fireEvent.keyDown(tab("babysitter"), { key: "Enter" });
  expect(tab("babysitter")).toHaveAttribute("aria-selected", "true");
});
