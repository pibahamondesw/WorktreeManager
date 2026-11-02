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
  onChanged.mockReset();
  vi.mocked(revealItemInDir).mockReset().mockResolvedValue(undefined);
  vi.mocked(openEditorForWorktree).mockReset().mockResolvedValue(null);
});

const onChanged = vi.fn();

function renderView(view: AutomationSummary = summary, editorApp: EditorApp = "cursor") {
  render(<AutomationDetail summary={view} editorApp={editorApp} onChanged={onChanged} />);
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

const runsPage = { runs: [], next_before: null };

function answer(responses: Record<string, unknown>) {
  vi.mocked(invoke).mockImplementation((command: string) =>
    command in responses
      ? responses[command] instanceof Error
        ? Promise.reject((responses[command] as Error).message)
        : Promise.resolve(responses[command])
      : Promise.resolve(runsPage)
  );
}

it("runs now, test runs and enables the automation, then reloads it", async () => {
  answer({
    automation_run: { run_id: "1761800000009-abcdef12", status: "starting", skip_reason: null },
    automation_resume: {},
  });
  renderView();

  fireEvent.click(screen.getByRole("button", { name: "Run now" }));
  expect(await screen.findByText("Run 1761800000009-abcdef12 started")).toBeInTheDocument();
  expect(invoke).toHaveBeenCalledWith("automation_run", {
    name: "pr-babysitter",
    test: false,
    payload: null,
  });
  expect(onChanged).toHaveBeenCalledTimes(1);

  answer({
    automation_run: {
      run_id: "1761800000010-abcdef12",
      status: "skipped",
      skip_reason: "already_running",
    },
  });
  fireEvent.click(screen.getByRole("button", { name: "Test run" }));
  expect(await screen.findByText("Test run skipped: already running")).toBeInTheDocument();
  expect(invoke).toHaveBeenCalledWith("automation_run", {
    name: "pr-babysitter",
    test: true,
    payload: null,
  });

  answer({ automation_resume: new Error("pr-babysitter has definition errors; fix them first") });
  fireEvent.click(screen.getByRole("button", { name: "Enable" }));
  expect(
    await screen.findByText("pr-babysitter has definition errors; fix them first")
  ).toHaveClass("select-text");
  expect(invoke).toHaveBeenCalledWith("automation_resume", {
    name: "pr-babysitter",
    by: "user",
    reason: null,
  });
  expect(onChanged).toHaveBeenCalledTimes(2);
});

it("pauses an enabled automation and says who last changed it", async () => {
  answer({ automation_pause: {} });
  renderView({
    ...summary,
    entry: {
      ...summary.entry,
      definition: { ...summary.entry.definition!, trigger: { manual: false, schedule: null } },
    },
    state: {
      enabled: true,
      pause_reason: null,
      next_run_at: null,
      consecutive_failures: 0,
      last_toggle: { enabled: true, by: "cli", reason: "back", at: "2026-10-30T12:00:00Z" },
    },
  });
  expect(screen.queryByRole("button", { name: "Run now" })).not.toBeInTheDocument();
  expect(screen.getByText(/^Resumed by the CLI on .*: back$/)).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Pause" }));
  await vi.waitFor(() => expect(onChanged).toHaveBeenCalled());
  expect(invoke).toHaveBeenCalledWith("automation_pause", {
    name: "pr-babysitter",
    by: "user",
    reason: null,
  });
});

it("resumes an automation paused by WTM and keeps invalid ones from running", () => {
  vi.mocked(invoke).mockResolvedValue(runsPage);
  renderView({
    ...summary,
    state: {
      ...summary.state!,
      pause_reason: "auto",
      last_toggle: { enabled: false, by: "auto", reason: null, at: "2026-10-30T12:00:00Z" },
    },
  });
  expect(screen.getByRole("button", { name: "Resume" })).toBeEnabled();
  expect(screen.getByText(/^Paused by WTM on /)).toBeInTheDocument();
  expect(screen.queryByText("Paused after repeated failures")).not.toBeInTheDocument();
  cleanup();

  renderView({
    ...summary,
    state: {
      ...summary.state!,
      pause_reason: "completed",
      last_toggle: { enabled: true, by: "user", reason: null, at: "2026-10-30T12:00:00Z" },
    },
  });
  expect(screen.getByText("Completed its runs")).toBeInTheDocument();
  cleanup();

  renderView({ ...summary, entry: { ...summary.entry, definition: null }, state: null });
  expect(screen.getByRole("button", { name: "Test run" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Enable" })).toBeDisabled();
});
