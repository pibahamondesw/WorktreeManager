// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
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

const renderHeader = (info?: IssueLinearInfo) =>
  render(
    <TaskHeader
      task={task}
      linearInfo={info}
      sessionStatus={{ kind: "running" }}
      sidebarCollapsed={false}
      onExpandSidebar={vi.fn()}
      onBack={vi.fn()}
    />
  );

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
