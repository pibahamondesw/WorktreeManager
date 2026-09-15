import { beforeEach, describe, expect, it, vi } from "vitest";
import { Task, TaskMember, Workspace } from "../types";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  openUrl: vi.fn(),
  fetchIssueLinearInfoBatch: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));
vi.mock("./linear", () => ({
  LinearService: class {
    fetchIssueLinearInfoBatch = mocks.fetchIssueLinearInfoBatch;
  },
}));

import { openCreatePr, openPrForMember } from "./pullRequest";

const member: TaskMember = {
  repoId: "r1",
  repoName: "web",
  localPath: "/repos/web",
  path: "/wt/web",
  branchName: "feat/palette",
};

const task: Task = {
  id: "t-1",
  workspaceId: "ws-1",
  branchName: "feat/palette",
  linearIssueId: "issue-1",
  members: [member],
  createdAt: "2026-01-01T00:00:00Z",
};

const workspace: Workspace = {
  id: "ws-1",
  name: "Payments",
  repos: [],
  linearApiKey: "lin_api_test",
};

beforeEach(() => {
  mocks.invoke.mockReset().mockResolvedValue("https://github.com/org/web");
  mocks.openUrl.mockReset().mockResolvedValue(undefined);
  mocks.fetchIssueLinearInfoBatch.mockReset();
});

describe("openCreatePr", () => {
  it("opens the GitHub compare URL for the branch", async () => {
    await expect(openCreatePr("feat/palette", member)).resolves.toBe(true);
    expect(mocks.invoke).toHaveBeenCalledWith("git_remote_url", { repoPath: "/repos/web" });
    expect(mocks.openUrl).toHaveBeenCalledWith(
      "https://github.com/org/web/compare/feat/palette?expand=1"
    );
  });
});

describe("openPrForMember", () => {
  it("opens the attached PR when the repo slug matches", async () => {
    mocks.fetchIssueLinearInfoBatch.mockResolvedValue({
      "issue-1": {
        status: null,
        prs: [
          {
            url: "https://github.com/org/web/pull/12",
            title: "Palette",
            state: "open",
            number: 12,
            repoSlug: "org/web",
          },
        ],
      },
    });
    await expect(openPrForMember(task, member, workspace)).resolves.toBe(true);
    expect(mocks.openUrl).toHaveBeenCalledWith("https://github.com/org/web/pull/12");
  });

  it("falls back to create-PR when Linear has no matching attachment", async () => {
    mocks.fetchIssueLinearInfoBatch.mockResolvedValue({
      "issue-1": { status: null, prs: [] },
    });
    await expect(openPrForMember(task, member, workspace)).resolves.toBe(true);
    expect(mocks.openUrl).toHaveBeenCalledWith(
      "https://github.com/org/web/compare/feat/palette?expand=1"
    );
  });
});
