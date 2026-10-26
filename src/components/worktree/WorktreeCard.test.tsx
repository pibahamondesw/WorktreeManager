// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  createEvent,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
} from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { OperationError } from "../../services/operations";
import { ensureTaskNote, taskNoteUri } from "../../services/notes";

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
import { useWorktreeListKeyboardShortcuts } from "../../hooks/useWorktreeListKeyboardShortcuts";
import { WorktreeCard, TaskCardActions } from "./WorktreeCard";
import { ComponentProps } from "react";
import * as taskLinear from "../../services/taskLinear";
import { Task, VaultConfig, Workspace } from "../../types";

type CardProps = Omit<ComponentProps<typeof WorktreeCard>, "actions"> & {
  onDelete: TaskCardActions["delete"];
  onOpen?: () => void;
  onOpenError?: (message: string) => void;
  onToast?: (message: string) => void;
  onLinkIssue?: () => void;
  onTogglePin?: () => void;
};

function Card({
  onDelete,
  onOpen,
  onOpenError,
  onToast,
  onLinkIssue,
  onTogglePin,
  ...props
}: CardProps) {
  const actions: TaskCardActions = {
    delete: onDelete,
    open: () => onOpen?.(),
    toast: (message) => onToast?.(message),
    reportError: (message) => onOpenError?.(message),
    deleteRequestHandled: () => undefined,
    linkIssue: onLinkIssue && (() => onLinkIssue()),
    togglePin: onTogglePin && (() => onTogglePin()),
  };
  return <WorktreeCard {...props} actions={actions} />;
}

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
      <Card
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
      <Card task={task} workspace={workspace} vault={vault} onDelete={vi.fn()} repoSlugs={{}} />
    );
    expect(screen.queryByTitle(/Agent session/)).not.toBeInTheDocument();
  });

  it("blinks only while working and marks a card waiting for input", () => {
    const view = render(
      <Card
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
      <Card
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
  const { rerender } = render(<Card {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Link Linear issue" }));
  expect(onLinkIssue).toHaveBeenCalledOnce();
  expect(onOpen).not.toHaveBeenCalled();
  rerender(
    <Card {...props} task={{ ...task, linearIssueId: "issue", linearIssueIdentifier: "WOR-123" }} />
  );
  expect(screen.queryByRole("button", { name: "Link Linear issue" })).not.toBeInTheDocument();
});

it.each([true, false])(
  "omits setup logs and dismissal controls from the card while active=%s",
  (active) => {
    initializeTaskSetup(task, []);
    updateTaskSetup(task.id, { ...getTaskSetup(task.id)!, active });
    render(
      <Card task={task} workspace={workspace} vault={vault} onDelete={vi.fn()} repoSlugs={{}} />
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
  const view = render(<Card {...props} />);
  fireEvent.click(screen.getByTitle("Pin task"));
  expect(onTogglePin).toHaveBeenCalledOnce();
  expect(onOpen).not.toHaveBeenCalled();
  view.rerender(<Card {...props} task={{ ...task, pinOrder: 0 }} />);
  expect(screen.getByTitle("Unpin task")).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByTitle("Drag to reorder pinned tasks")).toBeInTheDocument();
  fireEvent.click(screen.getByTitle("Unpin task"));
  expect(onTogglePin).toHaveBeenCalledTimes(2);
});

it("reports link failures without leaving a rejected promise", async () => {
  vi.mocked(openUrl).mockRejectedValueOnce(new Error("Browser unavailable"));
  const onToast = vi.fn();
  render(
    <Card
      task={{ ...task, linearIssueIdentifier: "WOR-129" }}
      workspace={workspace}
      vault={vault}
      onDelete={vi.fn()}
      onToast={onToast}
      repoSlugs={{}}
    />
  );
  fireEvent.click(screen.getByText("WOR-129"));
  await waitFor(() => expect(onToast).toHaveBeenCalledWith("Could not open Linear"));
});

it("reports clipboard failures without showing a success toast", async () => {
  const writeText = vi.fn().mockRejectedValue(new Error("Clipboard unavailable"));
  const previous = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  try {
    const onToast = vi.fn();
    render(
      <Card
        task={task}
        workspace={workspace}
        vault={vault}
        onDelete={vi.fn()}
        onToast={onToast}
        repoSlugs={{}}
      />
    );
    fireEvent.click(screen.getByTitle("Copy folder path(s)"));
    await waitFor(() => expect(onToast).toHaveBeenCalledWith("Could not copy to clipboard"));
    expect(onToast).not.toHaveBeenCalledWith("Worktree path copied");
  } finally {
    if (previous) Object.defineProperty(navigator, "clipboard", previous);
    else Reflect.deleteProperty(navigator, "clipboard");
  }
});

it("lists every attached PR by number with its full title on hover", () => {
  vi.mocked(openUrl).mockResolvedValueOnce(undefined);
  const onOpen = vi.fn();
  render(
    <Card
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
  expect(screen.getByTitle("#424: First")).toHaveTextContent(/^#424$/);
  expect(screen.getByTitle("#424: First").parentElement).toHaveTextContent(/^#424,$/);
  expect(screen.getByTitle("#425: Second")).toHaveTextContent(/^#425closed$/);
  expect(screen.queryByText(/First/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByTitle("#425: Second"));
  expect(openUrl).toHaveBeenCalledWith("https://x/pull/425");
  expect(onOpen).not.toHaveBeenCalled();
});

it("shows repository teardown scripts for approval and deletes with the chosen option", async () => {
  const approval = new OperationError("teardown_needs_approval", "Review the teardown", {
    scripts: [{ repoName: "api", source: "worktreemanager", script: "make stop" }],
  });
  const onDelete = vi
    .fn()
    .mockRejectedValueOnce(approval)
    .mockResolvedValue({ data: { id: task.id }, warnings: [] });
  render(
    <Card
      task={task}
      workspace={workspace}
      vault={vault}
      onDelete={onDelete}
      repoSlugs={{}}
      requestDelete
    />
  );
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  expect(await screen.findByText("make stop")).toBeInTheDocument();
  expect(screen.getByText("api · .worktreemanager.toml")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Run and delete" }));
  await waitFor(() =>
    expect(onDelete).toHaveBeenLastCalledWith(task.id, {
      deleteWorktrees: true,
      force: true,
      approveTeardown: true,
    })
  );
});

it("offers deleting without teardown after a teardown failure", async () => {
  const failure = new OperationError("teardown_failed", "Teardown failed for api.");
  const onDelete = vi
    .fn()
    .mockRejectedValueOnce(failure)
    .mockResolvedValue({ data: { id: task.id }, warnings: [] });
  render(
    <Card
      task={task}
      workspace={workspace}
      vault={vault}
      onDelete={onDelete}
      repoSlugs={{}}
      requestDelete
    />
  );
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  expect(await screen.findByText("Teardown failed for api.")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Run and delete" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Delete without teardown" }));
  await waitFor(() =>
    expect(onDelete).toHaveBeenLastCalledWith(task.id, {
      deleteWorktrees: true,
      force: true,
      skipTeardown: true,
    })
  );
});

it("offers removing from the app when disk deletion fails", async () => {
  const onDelete = vi
    .fn()
    .mockRejectedValueOnce(new OperationError("delete_failed", "Worktrees could not be removed."))
    .mockResolvedValue({ data: { id: task.id }, warnings: [] });
  render(
    <Card
      task={task}
      workspace={workspace}
      vault={vault}
      onDelete={onDelete}
      repoSlugs={{}}
      requestDelete
    />
  );
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  expect(await screen.findByText("Worktrees could not be removed.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Remove" }));
  await waitFor(() =>
    expect(onDelete).toHaveBeenLastCalledWith(task.id, { deleteWorktrees: false, force: false })
  );
});

it.each([false, true])(
  "offers a native PR button without opening the task for multiRepo=%s",
  async (multiRepo) => {
    vi.mocked(invoke).mockResolvedValue("https://github.com/example/api");
    vi.mocked(openUrl).mockClear();
    const onOpen = vi.fn();
    const onOpenTask = vi.fn().mockResolvedValue(true);
    renderHook(() =>
      useWorktreeListKeyboardShortcuts({
        workspace,
        vault,
        tasks: [task],
        selectedTask: task,
        showNew: false,
        searchOpen: false,
        taskOpen: false,
        setShowNew: vi.fn(),
        setSelectedIndex: vi.fn(),
        jumpToIndex: vi.fn(),
        setDeleteRequested: vi.fn(),
        handleRefresh: vi.fn(),
        showToast: vi.fn(),
        onOpenTask,
      })
    );
    const members = multiRepo
      ? [
          ...task.members,
          { ...task.members[0], repoId: "r2", repoName: "web", localPath: "/web", path: "/wt-web" },
        ]
      : task.members;
    render(
      <Card
        task={{ ...task, members }}
        workspace={workspace}
        vault={vault}
        onDelete={vi.fn()}
        repoSlugs={{}}
        onOpen={onOpen}
      />
    );
    const buttons = screen.getAllByRole("button", { name: "Create PR" });
    expect(buttons).toHaveLength(members.length);
    buttons[0].focus();
    expect(buttons[0]).toHaveFocus();
    expect(buttons[0].tagName).toBe("BUTTON");
    expect(buttons[0]).toHaveAttribute("type", "button");
    for (const key of ["Enter", " "]) {
      const event = createEvent.keyDown(buttons[0], { key, bubbles: true, cancelable: true });
      fireEvent(buttons[0], event);
      expect(event.defaultPrevented).toBe(false);
      expect(onOpenTask).not.toHaveBeenCalled();
    }
    fireEvent.click(buttons[0]);
    await waitFor(() =>
      expect(openUrl).toHaveBeenCalledWith("https://github.com/example/api/compare/feat/x?expand=1")
    );
    expect(onOpen).not.toHaveBeenCalled();
  }
);

it("copies paths from the keyboard with the card's message and never from text inputs", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  const previous = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  const input = document.body.appendChild(document.createElement("input"));
  try {
    const showToast = vi.fn();
    renderHook(() =>
      useWorktreeListKeyboardShortcuts({
        workspace,
        vault,
        tasks: [task],
        selectedTask: task,
        showNew: false,
        searchOpen: false,
        taskOpen: false,
        setShowNew: vi.fn(),
        setSelectedIndex: vi.fn(),
        jumpToIndex: vi.fn(),
        setDeleteRequested: vi.fn(),
        handleRefresh: vi.fn(),
        showToast,
        onOpenTask: vi.fn(),
      })
    );
    const copyPaths = { key: "C", code: "KeyC", metaKey: true, shiftKey: true, bubbles: true };

    fireEvent.keyDown(input, copyPaths);
    await Promise.resolve();
    expect(writeText).not.toHaveBeenCalled();

    fireEvent.keyDown(document.body, copyPaths);
    await waitFor(() => expect(showToast).toHaveBeenCalledWith("Worktree path copied"));
    expect(writeText).toHaveBeenCalledWith("/wt");
  } finally {
    input.remove();
    if (previous) Object.defineProperty(navigator, "clipboard", previous);
    else Reflect.deleteProperty(navigator, "clipboard");
  }
});

describe("WorktreeCard quick actions", () => {
  const linkedTask = { ...task, linearIssueIdentifier: "WOR-190" };
  const writeText = vi.fn();
  let previousClipboard: PropertyDescriptor | undefined;

  beforeEach(() => {
    previousClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    writeText.mockReset().mockResolvedValue(undefined);
    vi.mocked(openUrl).mockReset().mockResolvedValue(undefined);
    vi.mocked(ensureTaskNote).mockReset().mockResolvedValue("/vault/WOR-190.md");
    vi.mocked(taskNoteUri).mockImplementation((notePath) => `obsidian://open?path=${notePath}`);
  });

  afterEach(() => {
    if (previousClipboard) Object.defineProperty(navigator, "clipboard", previousClipboard);
    else Reflect.deleteProperty(navigator, "clipboard");
  });

  function renderCard() {
    const onOpen = vi.fn();
    const onToast = vi.fn();
    render(
      <Card
        task={linkedTask}
        workspace={{ ...workspace, linearOrgUrlKey: "acme" }}
        vault={{ ...vault, enabled: true }}
        onDelete={vi.fn()}
        repoSlugs={{}}
        onOpen={onOpen}
        onToast={onToast}
      />
    );
    return { onOpen, onToast };
  }

  it.each([
    { item: "Copy branch name", clipboard: "feat/x", toast: "Branch name copied" },
    { item: "Copy worktree path", clipboard: "/wt", toast: "Worktree path copied" },
    { item: "Open notes", url: "obsidian://open?path=/vault/WOR-190.md" },
  ])("runs $item from the menu and closes it", async ({ item, clipboard, url, toast }) => {
    const { onOpen, onToast } = renderCard();
    fireEvent.click(screen.getByTitle("More actions"));
    fireEvent.click(screen.getByText(item));

    expect(screen.queryByText(item)).not.toBeInTheDocument();
    if (clipboard) await waitFor(() => expect(writeText).toHaveBeenCalledWith(clipboard));
    if (url) await waitFor(() => expect(openUrl).toHaveBeenCalledWith(url));
    if (toast) expect(onToast).toHaveBeenCalledWith(toast);
    else expect(onToast).not.toHaveBeenCalled();
    expect(onOpen).not.toHaveBeenCalled();
  });

  it.each([
    { control: "Open on Linear", url: "https://linear.app/acme/issue/WOR-190" },
    { control: "Copy Linear ID", clipboard: "WOR-190", toast: "Linear ID copied" },
    { control: "Copy folder path(s)", clipboard: "/wt", toast: "Worktree path copied" },
    { control: "Copy branch name", clipboard: "feat/x", toast: "Branch name copied" },
  ])("runs $control without opening the task", async ({ control, clipboard, url, toast }) => {
    const { onOpen, onToast } = renderCard();
    fireEvent.click(screen.getByTitle(control));

    if (clipboard) await waitFor(() => expect(writeText).toHaveBeenCalledWith(clipboard));
    if (url) await waitFor(() => expect(openUrl).toHaveBeenCalledWith(url));
    if (toast) expect(onToast).toHaveBeenCalledWith(toast);
    expect(onOpen).not.toHaveBeenCalled();
  });
});

describe("WorktreeCard memoization", () => {
  const actions: TaskCardActions = {
    delete: vi.fn(),
    open: vi.fn(),
    toast: vi.fn(),
    reportError: vi.fn(),
    deleteRequestHandled: vi.fn(),
  };
  const status = { ahead: 1, behind: 0, dirty: false, last_commit_epoch: 1 };

  it("skips re-rendering when props are structurally unchanged", () => {
    const groupTaskPrs = vi.spyOn(taskLinear, "groupTaskPrs");
    const props = { task, workspace, vault, actions };
    const { rerender } = render(
      <WorktreeCard {...props} gitStatus={{ ...status }} repoSlugs={{ r1: "o/api" }} />
    );
    const rendersAfterMount = groupTaskPrs.mock.calls.length;
    rerender(<WorktreeCard {...props} gitStatus={{ ...status }} repoSlugs={{ r1: "o/api" }} />);
    expect(groupTaskPrs).toHaveBeenCalledTimes(rendersAfterMount);
    rerender(
      <WorktreeCard {...props} gitStatus={{ ...status, ahead: 2 }} repoSlugs={{ r1: "o/api" }} />
    );
    expect(groupTaskPrs.mock.calls.length).toBeGreaterThan(rendersAfterMount);
    groupTaskPrs.mockRestore();
  });
});
