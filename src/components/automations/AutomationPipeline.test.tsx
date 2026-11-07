// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { invoke } from "@tauri-apps/api/core";
import { AutomationPipeline } from "./AutomationPipeline";
import { AutomationDetail } from "./AutomationDetail";
import { OpenConversationContext } from "./openConversation";
import {
  AutomationItem,
  AutomationRun,
  AutomationSummary,
  PipelineStep,
  StepAttempt,
  StepReceipt,
} from "../../services/automations";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const STEPS: PipelineStep[] = [
  {
    id: "implement",
    label: "Implement",
    next: null,
    onFail: "fail",
    kind: {
      agent: {
        agent: { id: "claude", model: "opus", effort: "high" },
        skill: "implement",
        prompt: null,
        onReject: null,
      },
    },
  },
  {
    id: "verify",
    label: "verify",
    next: null,
    onFail: { goto: { step: "implement", max: { count: 3 } } },
    kind: { command: { command: ["scripts/verify", "--fast"], repo: "app" } },
  },
  { id: "checks", label: "checks", next: null, onFail: "block", kind: { wait: { signal: "ci" } } },
  {
    id: "approve",
    label: "approve",
    next: null,
    onFail: "fail",
    kind: { approval: { title: "Ship it?", detail: "Check the diff", onReject: "defer" } },
  },
  {
    id: "ship",
    label: "ship",
    next: null,
    onFail: "fail",
    kind: { publish: [{ effect: "git.push", repo: "app" }, { effect: "github.pr_create" }] },
  },
];

const attempt = (step: string, overrides: Partial<StepAttempt> = {}): StepAttempt => ({
  step,
  attempt: 1,
  status: "succeeded",
  process_end: null,
  exit_code: null,
  outcome: null,
  stage: null,
  verdict: null,
  cause: null,
  head: null,
  session_id: null,
  conversation_id: null,
  ...overrides,
});

const item = (id: string, overrides: Partial<AutomationItem> = {}): AutomationItem => ({
  id,
  title: null,
  status: "active",
  phase: null,
  steps: [],
  ...overrides,
});

const run = (id: string, overrides: Partial<AutomationRun> = {}): AutomationRun => ({
  id,
  trigger: { source: "manual", scheduled_for: null, test: false },
  status: "completed",
  skip_reason: null,
  failure_reason: null,
  outcome: "reported",
  claimed_at_ms: 1_000,
  ended_at_ms: 6_000,
  items: [],
  ...overrides,
});

const page = (...runs: AutomationRun[]) => ({ runs, next_before: null });

/** Answers the runs list with `runs` and every receipt read with `receipt`. */
const answer = (runs: ReturnType<typeof page>, receipt: StepReceipt | Error | null = null) =>
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command !== "automation_step_receipt") return runs;
    if (receipt instanceof Error) throw receipt.message;
    return receipt;
  });

const node = (label: string) => screen.getByRole("button", { name: new RegExp(`^${label}`) });

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});
beforeEach(() => vi.mocked(invoke).mockReset());

it("lists the permission requests the selected run captured", async () => {
  vi.mocked(invoke).mockResolvedValue(
    page(
      run("1761800000002-bbbbbbbb", {
        items: [item("routine", { steps: [attempt("implement")] })],
        permission_requests: [
          {
            step: "implement",
            attempt: 1,
            tool: "Bash",
            input: { command: "gh pr list" },
            resolution: "denied",
          },
          { step: "implement", attempt: 1, tool: "Allow?", input: "fetch", resolution: "allowed" },
          { step: "implement", attempt: 1, tool: "Ask", input: null, resolution: "denied" },
        ],
      })
    )
  );
  render(<AutomationPipeline name="babysitter" steps={STEPS} />);

  const section = await screen.findByRole("region", { name: "Permission requests" });
  expect(within(section).getByText('{"command":"gh pr list"}')).toHaveClass("font-mono");
  expect(within(section).getByText("fetch")).toBeInTheDocument();
  expect(within(section).getByText("—")).toBeInTheDocument();
  expect(within(section).getByText("allowed")).toBeInTheDocument();
});

it("opens the conversation of a step's latest agent attempt in the run's task", async () => {
  vi.mocked(invoke).mockResolvedValue(
    page(
      run("1761800000002-bbbbbbbb", {
        task_id: "t1",
        items: [
          item("routine", {
            steps: [attempt("implement", { session_id: "s1", conversation_id: "c1" })],
          }),
        ],
      })
    )
  );
  const openConversation = vi.fn().mockReturnValueOnce(true).mockReturnValueOnce(false);
  render(
    <OpenConversationContext.Provider value={openConversation}>
      <AutomationPipeline name="babysitter" steps={STEPS} />
    </OpenConversationContext.Provider>
  );
  fireEvent.click(await screen.findByRole("button", { name: /^Implement/ }));
  const open = screen.getByRole("button", { name: "Open conversation" });

  fireEvent.click(open);
  expect(openConversation).toHaveBeenCalledWith({
    taskId: "t1",
    agent: "claude",
    conversationId: "c1",
  });
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  fireEvent.click(open);
  expect(screen.getByRole("alert")).toHaveTextContent("The task of this run no longer exists");

  fireEvent.click(node("verify"));
  expect(screen.queryByRole("button", { name: "Open conversation" })).not.toBeInTheDocument();
});

it("shows the definition graph when there are no runs", async () => {
  vi.mocked(invoke).mockResolvedValue(page());
  const { container } = render(<AutomationPipeline name="babysitter" steps={STEPS} />);
  expect(await screen.findByText("No runs yet: showing the definition")).toBeInTheDocument();
  expect(within(node("Implement")).getByText("not started")).toBeInTheDocument();
  expect(within(node("Implement")).getByText("claude · implement")).toBeInTheDocument();
  expect(
    container.querySelector('[data-edge="verify → implement on fail (max 3)"]')
  ).not.toBeNull();
});

it("defaults to the newest run with items and shows each step's latest attempt", async () => {
  vi.mocked(invoke).mockResolvedValue(
    page(
      run("1761800000003-cccccccc", { status: "skipped", skip_reason: "paused" }),
      run("1761800000002-bbbbbbbb", {
        items: [
          item("pr-1", {
            title: "PR #1",
            phase: "verify",
            steps: [
              attempt("implement"),
              attempt("verify", { status: "failed", exit_code: 1 }),
              attempt("verify", { attempt: 2, status: "running" }),
            ],
          }),
          item("pr-2", {
            status: "completed",
            steps: [attempt("implement", { status: "rejected" })],
          }),
        ],
      })
    )
  );
  render(<AutomationPipeline name="babysitter" steps={STEPS} />);

  expect(await within(node("verify")).findByText("running")).toBeInTheDocument();
  expect(screen.getByRole("combobox", { name: "Run" })).toHaveValue("1761800000002-bbbbbbbb");
  expect(node("verify")).toHaveAttribute("aria-current", "step");
  expect(within(node("verify")).getByText("×2")).toBeInTheDocument();
  expect(within(node("Implement")).getByText("succeeded")).toBeInTheDocument();

  fireEvent.change(screen.getByRole("combobox", { name: "Item" }), { target: { value: "pr-2" } });
  expect(within(node("Implement")).getByText("rejected")).toBeInTheDocument();
  expect(within(node("verify")).getByText("not started")).toBeInTheDocument();

  fireEvent.change(screen.getByRole("combobox", { name: "Run" }), {
    target: { value: "1761800000003-cccccccc" },
  });
  expect(screen.queryByRole("combobox", { name: "Item" })).not.toBeInTheDocument();
  expect(within(node("Implement")).getByText("not started")).toBeInTheDocument();
});

it("moves between steps with the keyboard and opens what each kind has", async () => {
  answer(
    page(
      run("1761800000001-aaaaaaaa", {
        items: [
          item("only", {
            steps: [
              attempt("implement", { session_id: "s-1", conversation_id: "c-1" }),
              attempt("verify", { status: "failed", exit_code: 2, cause: "timeout" }),
              attempt("ship", {
                effects: [
                  {
                    effect: "git.push",
                    status: "staged",
                    detail: "Push wtm/x of app to origin",
                    reference: null,
                    warnings: [],
                  },
                  {
                    effect: "github.pr_create",
                    status: "executed",
                    detail: "Opened #7",
                    reference: "https://github.com/o/r/pull/7",
                    warnings: ["archive_note did not complete"],
                  },
                ],
              }),
            ],
          }),
        ],
      })
    )
  );
  render(
    <AutomationPipeline name="babysitter" steps={STEPS} repos={[{ id: "app", name: "web-app" }]} />
  );
  await within(node("verify")).findByText("failed");
  expect(screen.queryByRole("combobox", { name: "Item" })).not.toBeInTheDocument();

  const first = node("Implement");
  expect(first).toHaveAttribute("tabindex", "0");
  fireEvent.click(first);
  let detail = screen.getByRole("region", { name: "Step Implement" });
  expect(within(detail).getByText("claude · opus · high")).toBeInTheDocument();
  expect(within(detail).getByText("s-1")).toBeInTheDocument();
  expect(within(detail).getByText("c-1")).toBeInTheDocument();
  fireEvent.click(first);
  expect(screen.queryByRole("region")).not.toBeInTheDocument();

  first.focus();
  fireEvent.keyDown(first, { key: "ArrowUp" });
  expect(first).toHaveFocus();
  fireEvent.keyDown(first, { key: "ArrowDown" });
  expect(node("verify")).toHaveFocus();
  expect(node("verify")).toHaveAttribute("tabindex", "0");
  fireEvent.click(node("verify"));
  detail = screen.getByRole("region", { name: "Step verify" });
  expect(within(detail).getByText("scripts/verify --fast")).toBeInTheDocument();
  expect(within(detail).getByText("web-app")).toBeInTheDocument();
  expect(await within(detail).findByText("Log and receipt")).toBeInTheDocument();
  expect(within(detail).getByText("Not recorded for this run yet")).toBeInTheDocument();
  expect(within(detail).getByText("failed · attempt 1")).toBeInTheDocument();
  expect(within(detail).getByText("2")).toBeInTheDocument();
  expect(within(detail).getByText("timeout")).toBeInTheDocument();
  expect(within(detail).getByText("goto implement (max 3)")).toBeInTheDocument();

  fireEvent.keyDown(node("verify"), { key: "End" });
  expect(node("ship")).toHaveFocus();
  fireEvent.click(node("ship"));
  detail = screen.getByRole("region", { name: "Step ship" });
  expect(within(detail).getByText("git.push, github.pr_create")).toBeInTheDocument();
  expect(within(detail).getByText("1. git.push")).toBeInTheDocument();
  expect(within(detail).getByText("staged · Push wtm/x of app to origin")).toBeInTheDocument();
  expect(
    within(detail).getByText(
      "executed · Opened #7 · https://github.com/o/r/pull/7 · warning: archive_note did not complete"
    )
  ).toBeInTheDocument();
  fireEvent.keyDown(node("ship"), { key: "ArrowDown" });
  expect(node("ship")).toHaveFocus();
  fireEvent.keyDown(node("ship"), { key: "ArrowUp" });
  expect(node("approve")).toHaveFocus();
  fireEvent.click(node("approve"));
  detail = screen.getByRole("region", { name: "Step approve" });
  expect(within(detail).getByText("Check the diff")).toBeInTheDocument();
  expect(within(detail).getByText("Inbox entry")).toBeInTheDocument();
  expect(within(detail).getByText("defer")).toBeInTheDocument();

  fireEvent.keyDown(node("approve"), { key: "ArrowUp" });
  fireEvent.click(node("checks"));
  detail = screen.getByRole("region", { name: "Step checks" });
  expect(within(detail).getByText("Signal ci")).toBeInTheDocument();
  fireEvent.keyDown(node("checks"), { key: "Home" });
  expect(node("Implement")).toHaveFocus();
  fireEvent.keyDown(node("Implement"), { key: "a" });
  expect(node("Implement")).toHaveFocus();
});

const RECEIPT: StepReceipt = {
  receipt: {
    attempt: 1,
    process_end: "exited",
    exit_code: 1,
    duration_ms: 2_400,
    fingerprint: null,
  },
  reused: false,
  log: "1 offense detected\n",
  log_truncated: false,
};

const commandRun = (...steps: StepAttempt[]) =>
  page(run("1761800000001-aaaaaaaa", { items: [item("pr-1", { steps })] }));

it("shows a command attempt's receipt and the end of its log", async () => {
  answer(commandRun(attempt("verify", { status: "failed", exit_code: 1 })), RECEIPT);
  render(<AutomationPipeline name="babysitter" steps={STEPS} />);
  fireEvent.click(await screen.findByRole("button", { name: /^verify/ }));
  const detail = screen.getByRole("region", { name: "Step verify" });

  const log = await within(detail).findByLabelText("Command log");
  expect(log).toHaveTextContent("1 offense detected");
  expect(log).toHaveClass("font-mono", "select-text", "overflow-auto", "max-h-64");
  expect(within(detail).getByText("Log")).toBeInTheDocument();
  expect(within(detail).getByText("exit 1 · 2s")).toBeInTheDocument();
  expect(within(detail).getByText("Not bound")).toBeInTheDocument();
  expect(within(detail).getByText("No")).toBeInTheDocument();
  expect(invoke).toHaveBeenCalledWith("automation_step_receipt", {
    name: "babysitter",
    runId: "1761800000001-aaaaaaaa",
    item: "pr-1",
    step: "verify",
    attempt: 1,
  });
});

it("shows a reused receipt's fingerprint and a truncated log", async () => {
  answer(commandRun(attempt("verify", { attempt: 3, status: "skipped" })), {
    receipt: {
      ...RECEIPT.receipt,
      process_end: "timed_out",
      exit_code: null,
      fingerprint: {
        head: "0123456789abcdef",
        branch: "wtm/pr-1",
        index: "i",
        worktree: "fedcba9876543210",
      },
    },
    reused: true,
    log: "",
    log_truncated: true,
  });
  render(<AutomationPipeline name="babysitter" steps={STEPS} />);
  fireEvent.click(await screen.findByRole("button", { name: /^verify/ }));
  const detail = screen.getByRole("region", { name: "Step verify" });

  expect(await within(detail).findByText("Log (last 64 KB)")).toBeInTheDocument();
  expect(within(detail).getByLabelText("Command log")).toHaveTextContent("No output");
  expect(within(detail).getByText("timed out · 2s")).toBeInTheDocument();
  expect(within(detail).getByText("wtm/pr-1 · 0123456 · fedcba98")).toBeInTheDocument();
  expect(within(detail).getByText("Yes, from attempt 1")).toBeInTheDocument();
});

it("shows why a receipt could not be read", async () => {
  answer(
    commandRun(attempt("verify", { status: "failed" })),
    new Error("receipts/x.json is outside the run directory")
  );
  render(<AutomationPipeline name="babysitter" steps={STEPS} />);
  fireEvent.click(await screen.findByRole("button", { name: /^verify/ }));
  const detail = screen.getByRole("region", { name: "Step verify" });

  expect(
    await within(detail).findByText("receipts/x.json is outside the run directory")
  ).toHaveClass("text-danger");
  expect(within(detail).queryByLabelText("Command log")).not.toBeInTheDocument();
});

it("shows an approval's title as the run rendered it", async () => {
  answer(
    commandRun(attempt("approve", { status: "waiting", wait: { title: "Ship babysitter?" } }))
  );
  render(<AutomationPipeline name="babysitter" steps={STEPS} />);
  fireEvent.click(await screen.findByRole("button", { name: /^approve/ }));
  const detail = screen.getByRole("region", { name: "Step approve" });

  expect(within(detail).getByText("Ship babysitter?")).toBeInTheDocument();
  expect(within(detail).queryByText("Ship it?")).not.toBeInTheDocument();
  expect(invoke).not.toHaveBeenCalledWith("automation_step_receipt", expect.anything());
});

it("refreshes while the run is active and stops once it ends", async () => {
  vi.useFakeTimers();
  const live = run("1761800000001-aaaaaaaa", {
    status: "running",
    items: [
      item("only", { phase: "implement", steps: [attempt("implement", { status: "running" })] }),
    ],
  });
  vi.mocked(invoke)
    .mockResolvedValueOnce(page(live))
    .mockResolvedValueOnce(
      page({
        ...live,
        status: "completed",
        items: [item("only", { status: "completed", steps: [attempt("implement")] })],
      })
    );
  render(<AutomationPipeline name="babysitter" steps={STEPS} />);
  await act(async () => {});
  expect(screen.getByText("Live")).toBeInTheDocument();

  await act(async () => void vi.advanceTimersByTime(2000));
  expect(within(node("Implement")).getByText("succeeded")).toBeInTheDocument();
  expect(screen.queryByText("Live")).not.toBeInTheDocument();
  await act(async () => void vi.advanceTimersByTime(4000));
  expect(invoke).toHaveBeenCalledTimes(2);
});

it("shows load errors", async () => {
  vi.mocked(invoke).mockRejectedValueOnce("runs: permission denied");
  render(<AutomationPipeline name="babysitter" steps={STEPS} />);
  expect(await screen.findByText("runs: permission denied")).toHaveClass("select-text");
});

const summary = (definition: boolean): AutomationSummary => ({
  entry: {
    name: "babysitter",
    path: "/automations/babysitter/AUTOMATION.md",
    definition: definition
      ? {
          name: "babysitter",
          description: null,
          trigger: { manual: true, schedule: null },
          steps: STEPS,
        }
      : null,
    errors: definition
      ? []
      : [{ path: "/x", field: "steps", kind: "missing", message: "required" }],
    warnings: [],
  },
  state: null,
  stateError: null,
  latestRun: null,
});

it("is a tab of the automation detail", async () => {
  vi.mocked(invoke).mockResolvedValue(page());
  render(<AutomationDetail summary={summary(true)} editorApp="cursor" onChanged={() => {}} />);
  fireEvent.click(screen.getByRole("tab", { name: "Pipeline" }));
  expect(await screen.findByText("No runs yet: showing the definition")).toBeInTheDocument();
  expect(node("Implement")).toBeInTheDocument();

  cleanup();
  render(<AutomationDetail summary={summary(false)} editorApp="cursor" onChanged={() => {}} />);
  fireEvent.click(screen.getByRole("tab", { name: "Pipeline" }));
  expect(screen.getByText("Unavailable until the definition is valid")).toBeInTheDocument();
});
