// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { openUrl } from "@tauri-apps/plugin-opener";

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
    expect(screen.getByTitle("Agent working")).toHaveClass("bg-success", "motion-pulse-steps");
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
    expect(screen.getByTitle("Agent needs your input")).not.toHaveClass("motion-pulse-steps");
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

it("toggles the pin without opening the task and exposes its saved state", () => {
  const onTogglePin = vi.fn();
  const onOpen = vi.fn();
  const props = { task, workspace, vault, onDelete: vi.fn(), repoSlugs: {}, onTogglePin, onOpen };
  const view = render(<WorktreeCard {...props} />);
  fireEvent.click(screen.getByTitle("Pin task"));
  expect(onTogglePin).toHaveBeenCalledOnce();
  expect(onOpen).not.toHaveBeenCalled();
  view.rerender(<WorktreeCard {...props} task={{ ...task, pinOrder: 0 }} />);
  expect(screen.getByTitle("Unpin task")).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByTitle("Drag to reorder pinned tasks")).toBeInTheDocument();
  fireEvent.click(screen.getByTitle("Unpin task"));
  expect(onTogglePin).toHaveBeenCalledTimes(2);
});

it("reports link failures without leaving a rejected promise", async () => {
  vi.mocked(openUrl).mockRejectedValueOnce(new Error("Browser unavailable"));
  const onOpenError = vi.fn();
  render(
    <WorktreeCard
      task={{ ...task, linearIssueIdentifier: "WOR-129" }}
      workspace={workspace}
      vault={vault}
      onDelete={vi.fn()}
      onOpenError={onOpenError}
      repoSlugs={{}}
    />
  );
  fireEvent.click(screen.getByText("WOR-129"));
  await waitFor(() => expect(onOpenError).toHaveBeenCalledWith("Could not open Linear"));
});

it("reports clipboard failures without showing a success toast", async () => {
  const writeText = vi.fn().mockRejectedValue(new Error("Clipboard unavailable"));
  const previous = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  try {
    const onOpenError = vi.fn();
    const onToast = vi.fn();
    render(
      <WorktreeCard
        task={task}
        workspace={workspace}
        vault={vault}
        onDelete={vi.fn()}
        onOpenError={onOpenError}
        onToast={onToast}
        repoSlugs={{}}
      />
    );
    fireEvent.click(screen.getByTitle("Copy folder path(s)"));
    await waitFor(() => expect(onOpenError).toHaveBeenCalledWith("Could not copy to clipboard"));
    expect(onToast).not.toHaveBeenCalled();
  } finally {
    if (previous) Object.defineProperty(navigator, "clipboard", previous);
    else Reflect.deleteProperty(navigator, "clipboard");
  }
});

it("lists every attached PR by number with its full title on hover", () => {
  vi.mocked(openUrl).mockResolvedValueOnce(undefined);
  const onOpen = vi.fn();
  render(
    <WorktreeCard
      task={{ ...task, linearIssueId: "issue" }}
      workspace={workspace}
      vault={vault}
      onDelete={vi.fn()}
      onOpen={onOpen}
      repoSlugs={{}}
      linearInfo={
        {
          status: null,
          prs: [
            {
              url: "https://x/pull/424",
              title: "First",
              state: "open",
              number: 424,
              repoSlug: "o/r",
            },
            {
              url: "https://x/pull/425",
              title: "Second",
              state: "closed",
              number: 425,
              repoSlug: "o/r",
            },
          ],
        } as never
      }
    />
  );
  expect(screen.getByTitle("#424: First")).toHaveTextContent(/^#424open$/);
  expect(screen.getByTitle("#424: First").parentElement).toHaveTextContent(/^#424open,$/);
  expect(screen.getByTitle("#425: Second")).toHaveTextContent(/^#425closed$/);
  expect(screen.queryByText(/First/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByTitle("#425: Second"));
  expect(openUrl).toHaveBeenCalledWith("https://x/pull/425");
  expect(onOpen).not.toHaveBeenCalled();
});
