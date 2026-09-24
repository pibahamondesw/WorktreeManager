// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { WorktreeList } from "./WorktreeList";
import { Task } from "../../types";

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
  WorktreeCard: ({ task, onTogglePin }: { task: Task; onTogglePin: () => void }) => (
    <button onClick={onTogglePin}>{task.id}</button>
  ),
}));

afterEach(cleanup);

function fixture() {
  const onTaskPinned = vi.fn().mockResolvedValue(undefined);
  const onPinnedTasksReordered = vi.fn().mockResolvedValue(undefined);
  render(
    <WorktreeList
      tasks={[{ id: "first", pinOrder: 0 }, { id: "second", pinOrder: 1 }, { id: "unpinned" }].map(
        (item) => ({
          ...item,
          workspaceId: "w1",
          branchName: item.id,
          createdAt: "2026-01-01",
          members: [],
        })
      )}
      workspace={{ id: "w1", name: "Workspace", repos: [] }}
      vault={{ enabled: false, path: null }}
      onTaskCreated={vi.fn()}
      onTaskDeleted={vi.fn()}
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
      revealTaskId={null}
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
  return { onTaskPinned, onPinnedTasksReordered };
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
