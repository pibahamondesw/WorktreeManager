// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ClaudeUsageIndicator } from "./ClaudeUsageIndicator";
import { CodexUsageIndicator } from "./CodexUsageIndicator";
import { CachedUsage } from "../../hooks/useCachedUsage";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  suppress: vi.fn(),
  restore: vi.fn(),
  openUrl: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../services/codeEditor", () => ({
  editorPresentation: { suppress: mocks.suppress },
}));

function usageSnapshot(percent = 25): CachedUsage {
  return {
    usage: {
      supported: true,
      plan: null,
      account: null,
      windows: [
        { id: "five_hour", label: "5-hour limit", usedPercent: percent, resetsAt: 1790809800 },
        { id: "seven_day", label: "Weekly limit", usedPercent: 46, resetsAt: 1791108000 },
      ],
      session: null,
      error: null,
    },
    updatedAt: 1790800000000,
  };
}

describe.each([
  {
    label: "Claude",
    agentLabel: "Claude Code",
    Component: ClaudeUsageIndicator,
    command: "claude_cached_usage",
  },
  {
    label: "Codex",
    agentLabel: "Codex",
    Component: CodexUsageIndicator,
    command: "codex_cached_usage",
  },
])("$agentLabel usage indicator", ({ label, agentLabel, Component, command }) => {
  function snapshot(percent = 25): CachedUsage {
    const value = usageSnapshot(percent);
    if (label === "Codex") {
      value.usage!.windows[0].id = "primary";
      value.usage!.windows[1].id = "secondary";
    }
    return value;
  }

  async function mount() {
    await act(async () => render(<Component />));
    return screen.getByRole("button", { name: `${agentLabel} usage and limits` });
  }

  beforeEach(() => {
    vi.useFakeTimers();
    mocks.invoke.mockReset().mockResolvedValue(snapshot());
    mocks.openUrl.mockReset().mockResolvedValue(undefined);
    mocks.restore.mockReset();
    mocks.suppress.mockReset().mockReturnValue(mocks.restore);
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("labels the plan and provides accessible plan information for Codex", async () => {
    const value = snapshot();
    value.usage!.plan = "self_serve_business_prolite";
    mocks.invoke.mockResolvedValue(value);
    const trigger = await mount();
    await act(async () => fireEvent.click(trigger));
    expect(screen.getByText("Self Serve Business ProLite", { exact: false })).toHaveTextContent(
      "Plan: Self Serve Business ProLite"
    );
    const close = screen.getByRole("button", { name: `Close ${agentLabel} usage` });
    if (label === "Codex") {
      const link = screen.getByRole("link", { name: "More information about plans" });
      expect(link).toHaveAttribute("href", "https://learn.chatgpt.com/docs/pricing");
      fireEvent.keyDown(close, { key: "Tab" });
      expect(link).toHaveFocus();
      fireEvent.keyDown(link, { key: "Tab" });
      expect(close).toHaveFocus();
      fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
      expect(link).toHaveFocus();
      await act(async () => fireEvent.click(link));
      expect(mocks.openUrl).toHaveBeenCalledWith("https://learn.chatgpt.com/docs/pricing");
      mocks.openUrl.mockRejectedValueOnce(new Error("Browser unavailable"));
      await act(async () => fireEvent.click(link));
      expect(screen.getByRole("alert")).toHaveTextContent("Could not open plan information");
      await act(async () => fireEvent.click(link));
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    } else {
      expect(screen.queryByRole("link")).not.toBeInTheDocument();
    }
  });

  it("shows plan percentages and refreshes the detail on opening", async () => {
    const trigger = await mount();
    expect(trigger).toHaveTextContent(`${label}5h 25%7d 46%`);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    await act(async () => fireEvent.click(trigger));
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    expect(mocks.invoke).toHaveBeenLastCalledWith(command);
    expect(screen.getByRole("dialog", { name: `${agentLabel} usage` })).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "5-hour limit" })).toHaveAttribute(
      "aria-valuenow",
      "25"
    );
    expect(screen.getAllByText(/^Resets/)).toHaveLength(2);
    expect(screen.getByText(/^Last updated/)).toBeInTheDocument();
    expect(mocks.suppress).toHaveBeenCalledTimes(1);
  });

  it.each([
    [79, null],
    [80, "text-warning"],
    [95, "text-danger"],
  ])("alerts at %s percent", async (percent, expectedClass) => {
    mocks.invoke.mockResolvedValue(snapshot(percent as number));
    const trigger = await mount();
    if (expectedClass) {
      expect(trigger).toHaveClass(expectedClass);
      expect(screen.getByRole("status")).toHaveTextContent("Limit ≥80%");
    } else expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("updates percentages and warnings as the local cache changes", async () => {
    const trigger = await mount();
    await act(async () => fireEvent.click(trigger));
    mocks.invoke.mockResolvedValue(snapshot(86));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(trigger).toHaveTextContent("5h 86%");
    expect(screen.getByRole("status")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "5-hour limit" })).toHaveAttribute(
      "aria-valuenow",
      "86"
    );
    mocks.invoke.mockResolvedValue(snapshot(12));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("uses the reported duration instead of inferring it from the window ID", async () => {
    const value = snapshot(19);
    value.usage!.windows = [
      {
        id: value.usage!.windows[0].id,
        label: "Weekly limit",
        usedPercent: 19,
        resetsAt: 1791108000,
      },
    ];
    mocks.invoke.mockResolvedValue(value);
    const trigger = await mount();
    expect(trigger).toHaveTextContent(`${label}7d 19%`);
    expect(trigger).not.toHaveTextContent("5h");
  });

  it("shows unavailable data instead of a zero and recovers from read errors", async () => {
    mocks.invoke.mockResolvedValue({ usage: null, updatedAt: null });
    const trigger = await mount();
    expect(trigger).toHaveTextContent(`${label}—`);
    await act(async () => fireEvent.click(trigger));
    expect(screen.getByText(/No current usage data/)).toBeInTheDocument();
    mocks.invoke.mockRejectedValue(`Could not read ${agentLabel} usage cache`);
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(screen.getByText(`Could not read ${agentLabel} usage cache`)).toBeInTheDocument();
    mocks.invoke.mockResolvedValue(snapshot());
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(screen.getByText("25% used")).toBeInTheDocument();
    expect(screen.queryByText(`Could not read ${agentLabel} usage cache`)).not.toBeInTheDocument();
  });

  it("closes with Escape and outside clicks, restores focus and resumes the editor", async () => {
    const trigger = await mount();
    await act(async () => fireEvent.click(trigger));
    const close = screen.getByRole("button", { name: `Close ${agentLabel} usage` });
    expect(close).toHaveFocus();
    fireEvent.keyDown(close, { key: "Tab" });
    expect(close).toHaveFocus();
    const shortcut = vi.fn();
    window.addEventListener("keydown", shortcut);
    fireEvent.keyDown(close, { key: "Escape" });
    window.removeEventListener("keydown", shortcut);
    expect(shortcut).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
    expect(mocks.restore).toHaveBeenCalledTimes(1);
    await act(async () => fireEvent.click(trigger));
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("stops polling after unmounting and does not duplicate pending reads", async () => {
    let resolve: (value: CachedUsage) => void = () => undefined;
    mocks.invoke.mockReturnValue(
      new Promise<CachedUsage>((done) => {
        resolve = done;
      })
    );
    const trigger = await mount();
    await act(async () => fireEvent.click(trigger));
    await act(async () => vi.advanceTimersByTimeAsync(4000));
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    cleanup();
    await act(async () => resolve(snapshot()));
    await act(async () => vi.advanceTimersByTimeAsync(4000));
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });
});
