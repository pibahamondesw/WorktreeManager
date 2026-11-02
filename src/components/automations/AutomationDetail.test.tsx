// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { AutomationDetail } from "./AutomationDetail";
import { AutomationRun, AutomationSummary } from "../../services/automations";
import { openEditorForWorktree } from "../../services/openEditor";
import { EditorApp } from "../../types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));
vi.mock("../../services/openEditor", () => ({ openEditorForWorktree: vi.fn() }));

const PATH = "/home/.worktreemanager/automations/pr-babysitter/AUTOMATION.md";
const summary: AutomationSummary = {
  entry: {
    name: "pr-babysitter",
    path: PATH,
    definition: {
      name: "pr-babysitter",
      description: "Keeps PRs green",
      trigger: { manual: true, schedule: { cron: "0 * * * *", tz: "UTC", catchUp: "latest" } },
      steps: [],
    },
    errors: [],
    warnings: [
      { path: PATH, field: "budgets.active_time", kind: "invalid", message: "is unusually long" },
    ],
  },
  state: {
    enabled: false,
    pause_reason: "new",
    next_run_at: "2026-10-30T12:00:00Z",
    consecutive_failures: 0,
  },
  stateError: null,
  latestRun: null,
};
const run = (id: string, overrides: Partial<AutomationRun> = {}): AutomationRun => ({
  id,
  trigger: { source: "schedule", scheduled_for: null, test: false },
  status: "completed",
  skip_reason: null,
  failure_reason: null,
  outcome: "reported",
  claimed_at_ms: 1_000,
  ended_at_ms: 6_000,
  items: [],
  ...overrides,
});

afterEach(cleanup);
beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(revealItemInDir).mockReset().mockResolvedValue(undefined);
  vi.mocked(openEditorForWorktree).mockReset().mockResolvedValue(null);
});

function renderView(view: AutomationSummary = summary, editorApp: EditorApp = "cursor") {
  render(<AutomationDetail summary={view} editorApp={editorApp} />);
}

it("shows the trigger, pause reason and runs, and pages older runs", async () => {
  vi.mocked(invoke)
    .mockResolvedValueOnce({
      runs: [
        run("1761800000002-aaaaaaaa", {
          status: "skipped",
          skip_reason: "paused",
          trigger: { source: "schedule", scheduled_for: null, test: true },
        }),
      ],
      next_before: "1761800000002-aaaaaaaa",
    })
    .mockResolvedValueOnce({ runs: [run("1761800000001-bbbbbbbb")], next_before: null });
  renderView();

  expect(screen.getByText("Schedule 0 * * * * (UTC) · manual")).toBeInTheDocument();
  expect(screen.getByText("New automation, not enabled yet")).toBeInTheDocument();
  expect(screen.getByText("Not scheduled")).toBeInTheDocument();
  expect(await screen.findByText("Skipped: paused")).toBeInTheDocument();
  expect(screen.getByText("schedule · test")).toBeInTheDocument();
  expect(invoke).toHaveBeenCalledWith("automation_runs_list", {
    name: "pr-babysitter",
    before: null,
    limit: 50,
  });

  fireEvent.click(screen.getByRole("button", { name: "Load older runs" }));
  expect(await screen.findByText("Completed: reported")).toBeInTheDocument();
  expect(screen.getByText("Skipped: paused")).toBeInTheDocument();
  expect(screen.getAllByText("5s")).toHaveLength(2);
  expect(invoke).toHaveBeenLastCalledWith("automation_runs_list", {
    name: "pr-babysitter",
    before: "1761800000002-aaaaaaaa",
    limit: 50,
  });
  expect(screen.queryByRole("button", { name: "Load older runs" })).not.toBeInTheDocument();
});

it("reloads runs when the window regains focus and shows load errors", async () => {
  vi.mocked(invoke)
    .mockResolvedValueOnce({ runs: [], next_before: null })
    .mockRejectedValueOnce("runs: permission denied");
  renderView();
  expect(await screen.findByText("No runs yet")).toBeInTheDocument();
  act(() => void window.dispatchEvent(new Event("focus")));
  expect(await screen.findByText("runs: permission denied")).toHaveClass("select-text");
});

it("opens the definition in an external editor or reveals it in Finder", async () => {
  vi.mocked(invoke).mockResolvedValue({ runs: [], next_before: null });
  renderView();
  const runsTab = screen.getByRole("tab", { name: "Runs" });
  runsTab.focus();
  fireEvent.keyDown(runsTab, { key: "ArrowLeft" });
  const definitionTab = screen.getByRole("tab", { name: "Definition" });
  expect(definitionTab).toHaveAttribute("aria-selected", "true");
  expect(definitionTab).toHaveFocus();

  expect(screen.getByText(PATH)).toHaveClass("select-text");
  expect(screen.getByText("is unusually long")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Open in Cursor" }));
  const [editor, files, id, legacy, handlers] = vi.mocked(openEditorForWorktree).mock.calls[0];
  expect([editor, files, id, legacy]).toEqual(["cursor", [PATH], "automation-pr-babysitter", null]);
  act(() => handlers?.onError?.("Cursor is not installed"));
  expect(screen.getByText("Cursor is not installed")).toHaveClass("select-text");

  vi.mocked(revealItemInDir).mockRejectedValueOnce(new Error("denied"));
  fireEvent.click(screen.getByRole("button", { name: "Reveal in Finder" }));
  expect(revealItemInDir).toHaveBeenCalledWith(PATH);
  expect(await screen.findByText("Could not reveal the file in Finder")).toBeInTheDocument();
});

it("opens invalid automations on their errors and only reveals with embedded editors", () => {
  vi.mocked(invoke).mockResolvedValue({ runs: [], next_before: null });
  const issue = { path: PATH, field: "steps[0].agent", kind: "unknown_agent", message: "unknown" };
  renderView(
    {
      ...summary,
      entry: { ...summary.entry, definition: null, errors: [issue], warnings: [] },
      state: null,
      stateError: "state.json: expected value",
    },
    "vscode-web"
  );
  expect(screen.getByRole("tab", { name: "Definition" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByText("Invalid")).toBeInTheDocument();
  expect(screen.getByText("Unknown until the definition is valid")).toBeInTheDocument();
  expect(screen.getByText("state.json: expected value")).toBeInTheDocument();
  expect(screen.getByText("steps[0].agent:")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Open in/ })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Reveal in Finder" })).toBeInTheDocument();
});
