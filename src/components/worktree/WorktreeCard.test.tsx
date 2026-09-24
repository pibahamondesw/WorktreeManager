// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("../../services/notes", () => ({
  archiveTaskNote: vi.fn(),
  ensureTaskNote: vi.fn(),
  taskNoteUri: vi.fn(),
}));

import {
  clearTaskSetup,
  initializeTaskSetup,
  getTaskSetup,
  updateTaskSetup,
} from "../../services/taskSetup";
import { WorktreeCard } from "./WorktreeCard";
import { Task, VaultConfig, Workspace } from "../../types";

const workspace = { id: "ws1", name: "Payments", repos: [] } as unknown as Workspace;
const vault = { enabled: false } as VaultConfig;
const task = {
  id: "t1",
  workspaceId: "ws1",
  branchName: "feat/x",
  members: [
    { repoId: "r1", repoName: "api", localPath: "/repo", path: "/wt", branchName: "feat/x" },
  ],
  createdAt: "2026-01-01T00:00:00Z",
} as unknown as Task;

afterEach(() => {
  cleanup();
  clearTaskSetup(task.id);
});

describe("WorktreeCard session indicator", () => {
  it("shows a running dot when the task has a live agent session", () => {
    render(
      <WorktreeCard
        task={task}
        workspace={workspace}
        vault={vault}
        onDelete={vi.fn()}
        repoSlugs={{}}
        sessionStatus={{ kind: "running" }}
      />
    );
    expect(screen.getByTitle("Agent session active")).toBeInTheDocument();
  });

  it("shows nothing without a session", () => {
    render(
      <WorktreeCard
        task={task}
        workspace={workspace}
        vault={vault}
        onDelete={vi.fn()}
        repoSlugs={{}}
      />
    );
    expect(screen.queryByTitle(/Agent session/)).not.toBeInTheDocument();
  });

  it("blinks only while working and marks a card waiting for input", () => {
    const view = render(
      <WorktreeCard
        task={task}
        workspace={workspace}
        vault={vault}
        onDelete={vi.fn()}
        repoSlugs={{}}
        sessionStatus={{ kind: "running" }}
        agentActivity={{ state: "working", agent: "claude", surface: "chat", unread: false, at: 1 }}
      />
    );
    expect(screen.getByTitle("Agent working")).toHaveClass("bg-success", "animate-pulse");
    expect(view.container.querySelector("[data-attention]")).toBeNull();

    view.rerender(
      <WorktreeCard
        task={task}
        workspace={workspace}
        vault={vault}
        onDelete={vi.fn()}
        repoSlugs={{}}
        sessionStatus={{ kind: "running" }}
        agentActivity={{ state: "waiting", agent: "claude", surface: "chat", unread: true, at: 2 }}
      />
    );
    expect(screen.getByTitle("Agent needs your input")).toHaveClass("bg-warning");
    expect(screen.getByTitle("Agent needs your input")).not.toHaveClass("animate-pulse");
    expect(view.container.querySelector("[data-attention]")).not.toBeNull();
  });
});

it("offers linking only for an unlinked task without opening its editor", () => {
  const onLinkIssue = vi.fn();
  const onOpen = vi.fn();
  const props = { task, workspace, vault, onDelete: vi.fn(), repoSlugs: {}, onLinkIssue, onOpen };
  const { rerender } = render(<WorktreeCard {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Link Linear issue" }));
  expect(onLinkIssue).toHaveBeenCalledOnce();
  expect(onOpen).not.toHaveBeenCalled();
  rerender(
    <WorktreeCard
      {...props}
      task={{ ...task, linearIssueId: "issue", linearIssueIdentifier: "WOR-123" }}
    />
  );
  expect(screen.queryByRole("button", { name: "Link Linear issue" })).not.toBeInTheDocument();
});

it.each([true, false])(
  "omits setup logs and dismissal controls from the card while active=%s",
  (active) => {
    initializeTaskSetup(task, []);
    updateTaskSetup(task.id, { ...getTaskSetup(task.id)!, active });
    render(
      <WorktreeCard
        task={task}
        workspace={workspace}
        vault={vault}
        onDelete={vi.fn()}
        repoSlugs={{}}
      />
    );
    expect(screen.queryByText(/Setup running|Setup completed/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Dismiss setup progress" })
    ).not.toBeInTheDocument();
  }
);
