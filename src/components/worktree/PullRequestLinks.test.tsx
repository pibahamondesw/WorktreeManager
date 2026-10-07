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
