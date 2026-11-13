// @vitest-environment jsdom
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { GithubPrStatus, PullRequestInfo } from "../../types";
import { refreshGithubPrs, resetGithubCache, setPrReadyPending } from "../../services/github";
import { PullRequestLinks } from "./PullRequestLinks";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));
const pr: PullRequestInfo = {
  repoSlug: "org/repo",
  number: 1,
  state: "draft",
  title: "One",
  url: "https://github.com/org/repo/pull/1",
};
const status: GithubPrStatus = {
  state: "open",
  isDraft: true,
  ci: "running",
  review: "changes_requested",
};

beforeEach(() => {
  resetGithubCache();
  vi.mocked(invoke).mockReset();
});
afterEach(cleanup);

it("shares verified CI, review and draft with both rendered views", async () => {
  vi.mocked(invoke).mockResolvedValue({ 1: status });
  const onReady = vi.fn().mockResolvedValue(undefined);
  const parentClick = vi.fn();
  render(
    <div onClick={parentClick}>
      <PullRequestLinks prs={[pr]} onReady={onReady} />
      <PullRequestLinks prs={[pr]} />
    </div>
  );
  await act(() => refreshGithubPrs([pr]));
  expect(screen.getAllByText("CI running")).toHaveLength(2);
  expect(screen.getAllByText("Changes requested")).toHaveLength(2);
  expect(screen.getAllByText("draft")).toHaveLength(2);
  expect(screen.queryByText("open")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Mark as ready" }));
  expect(onReady).toHaveBeenCalledWith(pr);
  expect(parentClick).not.toHaveBeenCalled();
  act(() => setPrReadyPending(pr, true));
  expect(screen.getByRole("button", { name: "Marking ready…" })).toBeDisabled();
});

it("keeps existing links and hides ready when GitHub is unavailable", async () => {
  vi.mocked(invoke).mockRejectedValue(new Error("missing gh"));
  render(<PullRequestLinks prs={[pr]} onReady={vi.fn()} />);
  await act(() => refreshGithubPrs([pr]));
  expect(screen.getByText("draft")).toBeInTheDocument();
  expect(screen.queryByText("Mark as ready")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /#1/ }));
  expect(openUrl).toHaveBeenCalledWith(pr.url);
});

it("omits CI when no checks exist and hides active chips for merged PRs", async () => {
  vi.mocked(invoke).mockResolvedValue({
    1: { ...status, ci: "none", isDraft: false },
    2: { ...status, state: "merged", isDraft: false },
  });
  render(
    <PullRequestLinks prs={[pr, { ...pr, number: 2, url: `${pr.url}2` }]} onReady={vi.fn()} />
  );
  await act(() => refreshGithubPrs([pr, { ...pr, number: 2 }]));
  expect(screen.queryByText(/^CI /)).not.toBeInTheDocument();
  expect(screen.getAllByText("Changes requested")).toHaveLength(1);
  expect(screen.getByText("merged")).toBeInTheDocument();
  expect(screen.queryByText("Mark as ready")).not.toBeInTheDocument();
});

it("reports a ready failure without losing the draft", async () => {
  vi.mocked(invoke).mockResolvedValue({ 1: status });
  const onOpenError = vi.fn();
  render(
    <PullRequestLinks
      prs={[pr]}
      onReady={vi.fn().mockRejectedValue(new Error("denied"))}
      onOpenError={onOpenError}
    />
  );
  await act(() => refreshGithubPrs([pr]));
  await act(async () => {
    fireEvent.click(screen.getByText("Mark as ready"));
  });
  expect(onOpenError).toHaveBeenCalledWith("Error: denied");
  expect(screen.getByText("draft")).toBeInTheDocument();
});

it("omits the default open badge without GitHub data and preserves inactive states", () => {
  render(
    <PullRequestLinks
      prs={[
        { ...pr, state: "open" },
        { ...pr, number: 2, state: "closed", url: "https://github.com/org/repo/pull/2" },
        { ...pr, number: 3, state: "merged", url: "https://github.com/org/repo/pull/3" },
      ]}
    />
  );
  expect(screen.queryByText("open")).not.toBeInTheDocument();
  expect(screen.getByText("closed")).toBeInTheDocument();
  expect(screen.getByText("merged")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "#1" })).toBeInTheDocument();
});

it("shows a distinct state icon for draft, open, merged, closed and conflicting PRs", async () => {
  const numbers = [1, 2, 3, 4, 5];
  const prs = numbers.map((number) => ({ ...pr, number, url: `${pr.url}/${number}` }));
  vi.mocked(invoke).mockResolvedValue({
    1: status,
    2: { ...status, isDraft: false },
    3: { ...status, isDraft: false, state: "merged" },
    4: { ...status, isDraft: false, state: "closed" },
    5: { ...status, isDraft: false, hasConflicts: true },
  });
  render(<PullRequestLinks prs={prs} numbersOnly />);
  await act(() => refreshGithubPrs(prs));
  const titles = numbers.map((number) =>
    screen.getByRole("button", { name: `#${number}` }).getAttribute("title")
  );
  expect(titles).toEqual([
    "#1 (draft): One",
    "#2: One",
    "#3 (merged): One",
    "#4 (closed): One",
    "#5 (conflicts): One",
  ]);
  const icons = numbers.map(
    (number) => screen.getByRole("button", { name: `#${number}` }).querySelector("path")!.outerHTML
  );
  expect(new Set(icons).size).toBe(5);
});

it("loads checks only when the CI popover opens and opens their links", async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "github_pr_checks"
      ? [
          { name: "test", state: "failing", url: "https://ci/test" },
          { name: "docs", state: "skipped" },
        ]
      : { 1: { ...status, isDraft: false, ci: "failing" } }
  );
  const parentClick = vi.fn();
  render(
    <div onClick={parentClick}>
      <PullRequestLinks prs={[pr]} />
    </div>
  );
  await act(() => refreshGithubPrs([pr]));
  expect(invoke).not.toHaveBeenCalledWith("github_pr_checks", expect.anything());
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "CI failing" }));
  });
  expect(invoke).toHaveBeenCalledWith("github_pr_checks", { repoSlug: "org/repo", prNumber: 1 });
  const dialog = screen.getByRole("dialog", { name: "Checks for #1" });
  expect(dialog).toHaveTextContent("docs");
  fireEvent.click(screen.getByRole("button", { name: "test" }));
  expect(openUrl).toHaveBeenCalledWith("https://ci/test");
  expect(screen.queryByRole("button", { name: "docs" })).not.toBeInTheDocument();
  expect(parentClick).not.toHaveBeenCalled();
  fireEvent.keyDown(dialog, { key: "Escape" });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("shows the checks error inside the popover", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "github_pr_checks") throw "GitHub command timed out";
    return { 1: { ...status, isDraft: false, ci: "passing" } };
  });
  render(<PullRequestLinks prs={[pr]} />);
  await act(() => refreshGithubPrs([pr]));
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "CI passing" }));
  });
  expect(screen.getByRole("dialog")).toHaveTextContent("GitHub command timed out");
});

it("keeps popover interactions away from the card and closes when focus leaves", async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "github_pr_checks"
      ? [{ name: "test", state: "failing", url: "https://ci/test" }]
      : { 1: { ...status, isDraft: false, ci: "failing" } }
  );
  const parentMouseDown = vi.fn();
  render(
    <div onMouseDown={parentMouseDown}>
      <PullRequestLinks prs={[pr]} />
      <button type="button">elsewhere</button>
    </div>
  );
  await act(() => refreshGithubPrs([pr]));
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "CI failing" }));
  });
  const dialog = screen.getByRole("dialog");
  expect(dialog).toHaveFocus();
  fireEvent.mouseDown(screen.getByRole("button", { name: "test" }));
  expect(parentMouseDown).not.toHaveBeenCalled();
  act(() => screen.getByRole("button", { name: "test" }).focus());
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  act(() => screen.getByRole("button", { name: "elsewhere" }).focus());
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("ignores checks from a previous opening of the popover", async () => {
  const pending: ((checks: unknown) => void)[] = [];
  vi.mocked(invoke).mockImplementation((command) =>
    command === "github_pr_checks"
      ? new Promise((resolve) => pending.push(resolve))
      : Promise.resolve({ 1: { ...status, isDraft: false, ci: "failing" } })
  );
  render(<PullRequestLinks prs={[pr]} />);
  await act(() => refreshGithubPrs([pr]));
  const trigger = screen.getByRole("button", { name: "CI failing" });
  fireEvent.click(trigger);
  fireEvent.pointerDown(document.body);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  fireEvent.click(trigger);
  await act(async () => pending[1]([{ name: "fresh", state: "passing" }]));
  await act(async () => pending[0]([{ name: "stale", state: "failing" }]));
  expect(screen.getByRole("dialog")).toHaveTextContent("fresh");
  expect(screen.getByRole("dialog")).not.toHaveTextContent("stale");
});
