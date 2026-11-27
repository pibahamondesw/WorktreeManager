// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { UsageLimitBanner } from "./UsageLimitBanner";
import { ScheduledResume } from "../../services/chat";

const invoke = vi.hoisted(() => vi.fn<(command: string, args?: unknown) => Promise<unknown>>());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

afterEach(() => {
  cleanup();
  vi.resetAllMocks();
  vi.useRealTimers();
});

const inAnHour = Math.floor(Date.now() / 1000) + 3600;
const scheduled: ScheduledResume = {
  taskId: "t1",
  agent: "claude",
  folders: ["/wt"],
  conversationId: "c1",
  resumeAt: inAnHour,
};

function answer(commands: Record<string, () => Promise<unknown>>) {
  invoke.mockImplementation((command: string) => commands[command]?.() ?? Promise.resolve(null));
}

function banner(resetsAt: number | null, prompts = 1, onSnooze?: (until: string) => Promise<void>) {
  return (
    <UsageLimitBanner
      taskId="t1"
      agent="claude"
      agentLabel="Claude Code"
      tab="a1"
      folders={["/wt"]}
      resetsAt={resetsAt}
      prompts={prompts}
      onSnooze={onSnooze}
    />
  );
}

it("schedules the chat to continue when the limit renews and cancels it", async () => {
  answer({ chat_resume_schedule: () => Promise.resolve(scheduled) });
  render(banner(inAnHour));
  expect(screen.getByRole("status")).toHaveTextContent(/^Claude Code hit its usage limit\. Resets/);

  fireEvent.click(screen.getByRole("button", { name: "Resume when it renews" }));
  await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(/^Continues/));
  expect(invoke).toHaveBeenCalledWith("chat_resume_schedule", {
    taskId: "t1",
    agent: "claude",
    folders: ["/wt"],
    resumeAt: inAnHour,
    tab: "a1",
  });

  fireEvent.click(screen.getByRole("button", { name: "Cancel resume" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Resume when it renews" })).toBeEnabled()
  );
  expect(invoke).toHaveBeenCalledWith("chat_resume_cancel", {
    taskId: "t1",
    agent: "claude",
    tab: "a1",
  });
});

it("shows a resume scheduled before a restart until a prompt ends it", async () => {
  let found: ScheduledResume | null = scheduled;
  answer({ chat_resume_scheduled: () => Promise.resolve(found) });
  const { rerender } = render(banner(null));
  await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(/^Continues/));

  found = null;
  rerender(banner(null, 2));
  await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
});

it("snoozes until the limit renews and leaves a scheduled resume in place", async () => {
  answer({ chat_resume_scheduled: () => Promise.resolve(scheduled) });
  const onSnooze = vi.fn().mockResolvedValue(undefined);
  render(banner(inAnHour, 1, onSnooze));
  await screen.findByRole("button", { name: "Cancel resume" });

  const snooze = screen.getByRole("button", { name: "Snooze until it renews" });
  fireEvent.click(snooze);
  expect(snooze).toBeDisabled();
  expect(screen.getByRole("button", { name: "Cancel resume" })).toBeDisabled();
  await waitFor(() => expect(snooze).toBeEnabled());
  expect(onSnooze).toHaveBeenCalledExactlyOnceWith(new Date(inAnHour * 1000).toISOString());
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["chat_resume_scheduled"]);
  expect(screen.getByRole("button", { name: "Cancel resume" })).toBeEnabled();
});

it("shows a failed snooze and allows retrying without scheduling a resume", async () => {
  answer({});
  const onSnooze = vi
    .fn()
    .mockRejectedValueOnce(new Error("Could not save task"))
    .mockResolvedValue(undefined);
  render(banner(inAnHour, 1, onSnooze));

  const snooze = screen.getByRole("button", { name: "Snooze until it renews" });
  fireEvent.click(snooze);
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not save task");
  expect(snooze).toBeEnabled();
  fireEvent.click(snooze);
  await waitFor(() => expect(snooze).toBeEnabled());
  expect(onSnooze).toHaveBeenCalledTimes(2);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["chat_resume_scheduled"]);
});

it("ignores a snooze click if the limit renewed while the banner was visible", async () => {
  vi.useFakeTimers();
  answer({});
  const onSnooze = vi.fn();
  render(banner(inAnHour, 1, onSnooze));
  await act(() => vi.advanceTimersByTimeAsync(0));
  const snooze = screen.getByRole("button", { name: "Snooze until it renews" });

  vi.setSystemTime(inAnHour * 1000);
  await act(async () => {
    fireEvent.click(snooze);
  });

  expect(onSnooze).not.toHaveBeenCalled();
  expect(snooze).not.toBeInTheDocument();
});

it("stays hidden without a limit in force and reports a failed schedule", async () => {
  answer({
    chat_resume_scheduled: () => Promise.reject(new Error("unreadable")),
    chat_resume_schedule: () => Promise.reject("This chat has no conversation to continue yet"),
  });
  const { rerender } = render(banner(Math.floor(Date.now() / 1000) - 60));
  expect(screen.queryByRole("status")).not.toBeInTheDocument();

  rerender(banner(inAnHour));
  fireEvent.click(screen.getByRole("button", { name: "Resume when it renews" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "This chat has no conversation to continue yet"
  );
});

it("ignores a schedule read that finishes after the banner unmounts", async () => {
  let release: (value: ScheduledResume) => void = () => undefined;
  answer({
    chat_resume_scheduled: () => new Promise((resolve) => (release = resolve)),
  });
  const { unmount } = render(banner(null));
  unmount();
  release(scheduled);
  await Promise.resolve();
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});

it("checks again once the app has had time to take a due resume", async () => {
  vi.useFakeTimers();
  let found: ScheduledResume | null = { ...scheduled, resumeAt: Math.floor(Date.now() / 1000) };
  answer({ chat_resume_scheduled: () => Promise.resolve(found) });
  render(banner(null));
  await act(() => vi.advanceTimersByTimeAsync(0));
  expect(screen.getByRole("status")).toHaveTextContent(/^Continues/);

  found = null;
  await act(() => vi.advanceTimersByTimeAsync(35_000));
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
