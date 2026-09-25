// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { WorktreeList } from "./WorktreeList";
import type { ComponentProps } from "react";
import { Task } from "../../types";
import { DeleteOptions } from "../../services/operations";

type WorktreeListProps = ComponentProps<typeof WorktreeList>;
import { useWorktreeListKeyboardShortcuts } from "../../hooks/useWorktreeListKeyboardShortcuts";

vi.mock("../../hooks/useWorktreeData", () => ({
  useWorktreeData: () => ({
    linearInfo: {},
    gitStatuses: {},
    refreshing: false,
    handleRefresh: vi.fn(),
  }),
}));
vi.mock("../../hooks/useRepoSlugs", () => ({ useRepoSlugs: () => ({}) }));
vi.mock("../../hooks/useWorktreeListKeyboardShortcuts", () => ({
  useWorktreeListKeyboardShortcuts: vi.fn(),
}));
vi.mock("./WorktreeListHeader", () => ({ WorktreeListHeader: () => null }));
vi.mock("./NewWorktreeModal", () => ({ NewWorktreeModal: () => null }));
vi.mock("../task/TaskView", () => ({ TaskView: () => null }));
vi.mock("./WorktreeCard", () => ({
  WorktreeCard: ({
    task,
    onTogglePin,
    onDelete,
  }: {
    task: Task;
    onTogglePin: () => void;
    onDelete: (id: string, options: DeleteOptions) => Promise<unknown>;
  }) => (
    <>
      <button onClick={onTogglePin}>{task.id}</button>
      <button
        data-genie-target
        onClick={() =>
          void onDelete(task.id, { deleteWorktrees: true, force: true }).catch(() => undefined)
        }
      >
        delete {task.id}
      </button>
    </>
  ),
}));

afterEach(cleanup);

const TASKS: Task[] = [
  { id: "first", pinOrder: 0 },
  { id: "second", pinOrder: 1 },
  { id: "unpinned" },
].map((item) => ({
  ...item,
  workspaceId: "w1",
  branchName: item.id,
  createdAt: "2026-01-01",
  members: [],
}));

function fixture({
  revealTaskId = null,
  onTaskDeleted = vi.fn<WorktreeListProps["onTaskDeleted"]>(),
}: {
  revealTaskId?: string | null;
  onTaskDeleted?: WorktreeListProps["onTaskDeleted"];
} = {}) {
  const onTaskPinned = vi.fn().mockResolvedValue(undefined);
  const onPinnedTasksReordered = vi.fn().mockResolvedValue(undefined);
  const listProps = (tasks: Task[]) => (
    <WorktreeList
      tasks={tasks}
      workspace={{ id: "w1", name: "Workspace", repos: [] }}
      vault={{ enabled: false, path: null }}
      onTaskCreated={vi.fn()}
      onTaskDeleted={onTaskDeleted}
      onTaskIssueLinked={vi.fn()}
      onTaskPinned={onTaskPinned}
      onPinnedTasksReordered={onPinnedTasksReordered}
      editorApp="cursor"
      onEditorChange={vi.fn()}
      workspaceSwitching={false}
      onWorkspaceReady={vi.fn()}
      onOpenSearch={vi.fn()}
      sidebarCollapsed={false}
      onExpandSidebar={vi.fn()}
      searchOpen={false}
      revealTaskId={revealTaskId}
      onRevealHandled={vi.fn()}
      openedTask={null}
      onOpenTask={vi.fn()}
      onCloseTask={vi.fn()}
      onSwitchSurface={vi.fn()}
      canGoBack={false}
      canGoForward={false}
      onGoBack={vi.fn()}
      onGoForward={vi.fn()}
    />
  );
  const view = render(listProps(TASKS));
  const setTasks = (tasks: Task[]) => view.rerender(listProps(tasks));
  return { onTaskPinned, onPinnedTasksReordered, setTasks };
}

it("routes pin toggles and only allows pinned tasks to be dragged", async () => {
  const { onTaskPinned } = fixture();
  fireEvent.click(screen.getByText("unpinned"));
  fireEvent.click(screen.getByText("first"));
  await waitFor(() =>
    expect(onTaskPinned.mock.calls).toEqual([
      ["unpinned", true],
      ["first", false],
    ])
  );
  expect(screen.getByText("unpinned").parentElement).toHaveAttribute("draggable", "false");
  expect(screen.getByText("first").parentElement).toHaveAttribute("draggable", "true");
});

it("reorders by task ID and ignores unpinned or external drops", async () => {
  const { onPinnedTasksReordered } = fixture();
  const first = screen.getByText("first").parentElement!;
  const second = screen.getByText("second").parentElement!;
  const unpinned = screen.getByText("unpinned").parentElement!;
  const dataTransfer = { setData: vi.fn(), effectAllowed: "", dropEffect: "" };
  fireEvent.drop(first, { dataTransfer });
  expect(onPinnedTasksReordered).not.toHaveBeenCalled();
  fireEvent.dragStart(first, { dataTransfer });
  fireEvent.dragOver(second, { dataTransfer });
  fireEvent.drop(second, { dataTransfer });
  await waitFor(() => expect(onPinnedTasksReordered).toHaveBeenCalledWith("first", "second"));
  fireEvent.dragStart(second, { dataTransfer });
  fireEvent.drop(unpinned, { dataTransfer });
  expect(onPinnedTasksReordered).toHaveBeenCalledOnce();
});

it("shows persistence errors from reordering", async () => {
  const { onPinnedTasksReordered } = fixture();
  onPinnedTasksReordered.mockRejectedValue(new Error("Could not save changes"));
  const dataTransfer = { setData: vi.fn() };
  fireEvent.dragStart(screen.getByText("first").parentElement!, { dataTransfer });
  fireEvent.drop(screen.getByText("second").parentElement!, { dataTransfer });
  expect(await screen.findByText("Error: Could not save changes")).toBeInTheDocument();
});

function revealRingOwner() {
  return screen.getByTestId("reveal-ring").nextElementSibling!.textContent;
}

it("rings the card revealed from search or history", () => {
  fixture({ revealTaskId: "second" });
  expect(revealRingOwner()).toBe("second");
  const ring = screen.getByTestId("reveal-ring");
  fireEvent(ring, new Event("webkitAnimationEnd", { bubbles: true }));
  expect(screen.queryByTestId("reveal-ring")).not.toBeInTheDocument();
});

it("rings the card jumped to by number", () => {
  fixture();
  expect(screen.queryByTestId("reveal-ring")).not.toBeInTheDocument();
  const { jumpToIndex } = vi.mocked(useWorktreeListKeyboardShortcuts).mock.lastCall![0];
  act(() => jumpToIndex(2));
  expect(revealRingOwner()).toBe("unpinned");
});

function deferredDeletion() {
  const deferred = {} as {
    resolve: (value: { data: { id: string }; warnings: [] }) => void;
    reject: (error: Error) => void;
  };
  const onTaskDeleted = vi.fn<WorktreeListProps["onTaskDeleted"]>(
    () => new Promise((resolve, reject) => Object.assign(deferred, { resolve, reject }))
  );
  return { deferred, onTaskDeleted };
}

it("dims a deleting card and restores it with a toast when deletion fails", async () => {
  const { deferred, onTaskDeleted } = deferredDeletion();
  fixture({ onTaskDeleted });
  const card = screen.getByText("second").parentElement!;
  fireEvent.click(screen.getByText("delete second"));
  expect(onTaskDeleted).toHaveBeenCalledWith("second", { deleteWorktrees: true, force: true });
  expect(card).toHaveClass("opacity-50");
  expect(card).toHaveAttribute("inert");
  expect(screen.getByText("first").parentElement).not.toHaveAttribute("inert");
  await act(async () => deferred.reject(new Error("git worktree remove failed")));
  expect(card).not.toHaveClass("opacity-50");
  expect(card).not.toHaveAttribute("inert");
  expect(screen.getByText("git worktree remove failed")).toBeInTheDocument();
});

it("removes a deleted card once its exit animation finishes", async () => {
  const { deferred, onTaskDeleted } = deferredDeletion();
  const { setTasks } = fixture({ onTaskDeleted });
  fireEvent.click(screen.getByText("delete second"));
  setTasks(TASKS.filter((task) => task.id !== "second"));
  await act(async () => deferred.resolve({ data: { id: "second" }, warnings: [] }));
  expect(screen.queryByText("second")).not.toBeInTheDocument();
});

it("does not animate tasks removed by something other than a card deletion", () => {
  const { setTasks } = fixture();
  setTasks(TASKS.filter((task) => task.id !== "second"));
  expect(screen.queryByText("second")).not.toBeInTheDocument();
});

it("plays the exit even when deletion settles before the task list updates", async () => {
  const animate = vi.fn(() => ({ cancel: vi.fn(), onfinish: null }) as unknown as Animation);
  Element.prototype.animate = animate;
  try {
    const { deferred, onTaskDeleted } = deferredDeletion();
    const { setTasks } = fixture({ onTaskDeleted });
    fireEvent.click(screen.getByText("delete second"));
    await act(async () => deferred.resolve({ data: { id: "second" }, warnings: [] }));
    setTasks(TASKS.filter((task) => task.id !== "second"));
    expect(screen.getByText("second").closest("[inert]")).not.toBeNull();
    expect(screen.getAllByText(/^(first|second|unpinned)$/).map((el) => el.textContent)).toEqual([
      "first",
      "second",
      "unpinned",
    ]);
  } finally {
    delete (Element.prototype as Partial<Element>).animate;
  }
});

it("keeps the grid for the last card's exit instead of jumping to the empty state", async () => {
  Element.prototype.animate = vi.fn(
    () => ({ cancel: vi.fn(), onfinish: null }) as unknown as Animation
  );
  try {
    const { deferred, onTaskDeleted } = deferredDeletion();
    const { setTasks } = fixture({ onTaskDeleted });
    setTasks(TASKS.slice(0, 1));
    fireEvent.click(screen.getByText("delete first"));
    setTasks([]);
    await act(async () => deferred.resolve({ data: { id: "first" }, warnings: [] }));
    expect(screen.getByText("first").closest("[inert]")).not.toBeNull();
  } finally {
    delete (Element.prototype as Partial<Element>).animate;
  }
});

it("filters cards and keyboard targets by project and restores all tasks", () => {
  const { setTasks } = fixture();
  setTasks(
    TASKS.map((task, index) =>
      index === 1 ? { ...task, linearProjectId: "p1", linearProjectName: "API" } : task
    )
  );
  const filter = screen.getByRole("combobox", { name: "Linear project" });
  fireEvent.change(filter, { target: { value: "p1" } });
  expect(screen.queryByText("first")).not.toBeInTheDocument();
  expect(screen.getByText("second")).toBeInTheDocument();
  expect(
    vi.mocked(useWorktreeListKeyboardShortcuts).mock.lastCall?.[0].tasks.map((task) => task.id)
  ).toEqual(["second"]);
  fireEvent.change(filter, { target: { value: "__no_project__" } });
  expect(screen.queryByText("second")).not.toBeInTheDocument();
  expect(screen.getByText("first")).toBeInTheDocument();
  fireEvent.change(filter, { target: { value: "" } });
  expect(screen.getByText("second")).toBeInTheDocument();
});
