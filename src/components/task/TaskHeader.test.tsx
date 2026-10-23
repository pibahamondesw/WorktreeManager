// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { openUrl } from "@tauri-apps/plugin-opener";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

import { TaskHeader } from "./TaskHeader";
import { IssueLinearInfo, Task } from "../../types";

const task = {
  id: "t1",
  branchName: "feat/x",
  linearIssueId: "issue",
  linearIssueIdentifier: "WOR-79",
  members: [],
} as unknown as Task;

const linearInfo: IssueLinearInfo = {
  status: null,
  prs: [
    {
      url: "https://github.com/o/r/pull/424",
      title: "First",
      state: "open",
      number: 424,
      repoSlug: "o/r",
    },
    {
      url: "https://github.com/o/r/pull/425",
      title: "Second",
      state: "merged",
      number: 425,
      repoSlug: "o/r",
    },
  ],
} as unknown as IssueLinearInfo;

afterEach(cleanup);
beforeEach(() => vi.mocked(openUrl).mockReset().mockResolvedValue(undefined));

const renderHeader = (info?: IssueLinearInfo, orgUrlKey?: string | null, headerTask = task) =>
  render(
    <TaskHeader
      task={headerTask}
      linearInfo={info}
      linearOrgUrlKey={orgUrlKey}
      sessionStatus={{ kind: "running" }}
      sidebarCollapsed={false}
      onExpandSidebar={vi.fn()}
      onBack={vi.fn()}
    />
  );

it.each([
  [undefined, "https://linear.app/issue/WOR-79"],
  ["my-org", "https://linear.app/my-org/issue/WOR-79"],
])("opens the Linear issue with organization %s before issue details load", (orgUrlKey, url) => {
  renderHeader(undefined, orgUrlKey);
  const link = screen.getByRole("link", { name: "WOR-79" });
  expect(link).toHaveAttribute("href", url);
  expect(fireEvent.click(link)).toBe(false);
  expect(openUrl).toHaveBeenCalledExactlyOnceWith(url);
});

it("handles modified and middle clicks without navigating the webview", () => {
  renderHeader();
  const link = screen.getByRole("link", { name: "WOR-79" });
  expect(fireEvent.click(link, { metaKey: true })).toBe(false);
  expect(
    fireEvent(link, new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }))
  ).toBe(false);
  fireEvent(link, new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 2 }));
  expect(openUrl).toHaveBeenCalledTimes(2);
});

it("reports opening failures and lets the user retry", async () => {
  vi.mocked(openUrl).mockRejectedValueOnce(new Error("Browser unavailable"));
  renderHeader();
  fireEvent.click(screen.getByRole("link", { name: "WOR-79" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not open Linear");
  fireEvent.click(screen.getByRole("link", { name: "WOR-79" }));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(openUrl).toHaveBeenCalledTimes(2);
});

it("omits the Linear link for tasks without an issue", () => {
  renderHeader(undefined, "my-org", {
    ...task,
    linearIssueId: undefined,
    linearIssueIdentifier: undefined,
  });
  expect(screen.queryByRole("link")).not.toBeInTheDocument();
});

it("links every attached PR by number with its full title on hover", () => {
  renderHeader(linearInfo);
  expect(screen.getByTitle("#424: First")).toHaveTextContent("#424");
  fireEvent.click(screen.getByTitle("#425: Second"));
  expect(openUrl).toHaveBeenCalledWith("https://github.com/o/r/pull/425");
});

it("omits the PR links when the issue has none", () => {
  renderHeader({ ...linearInfo, prs: [] });
  expect(screen.queryByTitle(/^#\d+:/)).not.toBeInTheDocument();
});
