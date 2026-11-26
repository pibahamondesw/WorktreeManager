// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, onTestFinished, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { WorktreeList, TaskActions } from "./WorktreeList";
import type { ReactNode } from "react";
import { Task } from "../../types";
import type { TaskCardActions } from "./WorktreeCard";
import { useWorktreeListKeyboardShortcuts } from "../../hooks/useWorktreeListKeyboardShortcuts";
import type { OpenedTask } from "../../hooks/useOpenTask";
import { loadShowAutomationTasks, persist } from "../../services/store";

vi.mock("../../services/store", () => ({
  loadShowAutomationTasks: vi.fn(),
  persist: vi.fn(),
}));

vi.mock("../../hooks/useWorktreeData", () => ({
  useWorktreeData: () => ({
    linearInfo: {},
    gitStatuses: {},
    refreshing: false,
    handleRefresh: vi.fn(),
  }),
}));
vi.mock("../../hooks/useWorktreeListKeyboardShortcuts", () => ({
  useWorktreeListKeyboardShortcuts: vi.fn(),
}));
vi.mock("./WorktreeListHeader", () => ({
  WorktreeListHeader: ({ projectFilter }: { projectFilter?: ReactNode }) => projectFilter,
}));
vi.mock("./NewWorktreeModal", () => ({ NewWorktreeModal: () => null }));
vi.mock("../task/TaskView", () => ({
  TaskView: ({ onBack }: { onBack: () => void }) => <button onClick={onBack}>Back</button>,
}));
vi.mock("./WorktreeCard", () => ({
  WorktreeCard: ({
    task,
    actions,
    selected,
  }: {
    task: Task;
    actions: TaskCardActions;
    selected: boolean;
  }) => (
    <>
      <button
        data-selected={selected ? "true" : undefined}
        onClick={() => actions.togglePin?.(task)}
      >
        {task.id}
      </button>
      <button
        data-genie-target
        onClick={() =>
          void actions
            .delete(task.id, { deleteWorktrees: true, force: true })
            .catch(() => undefined)
        }
      >
        delete {task.id}
      </button>
      <button onClick={() => actions.toggleShelved?.(task)}>shelve {task.id}</button>
      <button onClick={() => actions.snooze?.(task, LATER)}>snooze {task.id}</button>
      <button onClick={() => actions.pickSnoozeDate?.(task)}>pick snooze {task.id}</button>
    </>
  ),
}));

const LATER = "2099-01-01T00:00:00.000Z";

beforeEach(() => {
  vi.mocked(loadShowAutomationTasks).mockReset().mockResolvedValue(true);
  vi.mocked(persist).mockReset().mockResolvedValue(undefined);
});

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
  tasks = TASKS,
  revealTaskId = null,
  openedTask = null,
  onTaskDeleted = vi.fn<TaskActions["delete"]>(),
}: {
  tasks?: Task[];
  revealTaskId?: string | null;
  openedTask?: OpenedTask | null;
  onTaskDeleted?: TaskActions["delete"];
} = {}) {
  const onTaskPinned = vi.fn().mockResolvedValue(undefined);
  const onTaskShelved = vi.fn().mockResolvedValue(undefined);
  const onTaskSnoozed = vi.fn().mockResolvedValue(undefined);
  const onPinnedTasksReordered = vi.fn().mockResolvedValue(undefined);
  const listProps = (tasks: Task[], workspaceId = "w1") => (
    <WorktreeList
      tasks={tasks}
      workspace={{ id: workspaceId, name: "Workspace", repos: [] }}
      vault={{ enabled: false, path: null }}
      taskActions={{
        create: vi.fn(),
        delete: onTaskDeleted,
        linkIssue: vi.fn(),
        rename: vi.fn(),
        setPinned: onTaskPinned,
        setShelved: onTaskShelved,
        snooze: onTaskSnoozed,
        reorderPinned: onPinnedTasksReordered,
      }}
      editorApp="cursor"
      onEditorChange={vi.fn()}
      workspaceSwitching={false}
      onWorkspaceReady={vi.fn()}
      navigation={{
        canGoBack: false,
        canGoForward: false,
        goBack: vi.fn(),
        goForward: vi.fn(),
        sidebarCollapsed: false,
        expandSidebar: vi.fn(),
        openSearch: vi.fn(),
      }}
      searchOpen={false}
      revealTaskId={revealTaskId}
      onRevealHandled={vi.fn()}
      openedTask={openedTask}
      onOpenTask={vi.fn()}
      onCloseTask={vi.fn()}
      onSwitchSurface={vi.fn()}
    />
  );
  const view = render(listProps(tasks));
  const setTasks = (tasks: Task[]) => view.rerender(listProps(tasks));
  const switchWorkspace = (workspaceId: string, tasks: Task[]) =>
    view.rerender(listProps(tasks, workspaceId));
  return {
    onTaskPinned,
    onTaskShelved,
    onTaskSnoozed,
    onPinnedTasksReordered,
    setTasks,
    switchWorkspace,
  };
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
  expect(screen.getByText("unpinned").parentElement).not.toHaveAttribute("data-reorder-key");
  expect(screen.getByText("first").parentElement).toHaveAttribute("data-reorder-key", "first");
});

function drag(source: Element, target: Element) {
  fireEvent.mouseDown(source, { button: 0, clientX: 0, clientY: 0 });
  fireEvent.mouseMove(target, { clientX: 0, clientY: 40 });
  fireEvent.mouseUp(target);
}

it("reorders by task ID and ignores clicks, short moves and unpinned targets", async () => {
  const { onPinnedTasksReordered } = fixture();
  const first = screen.getByText("first").parentElement!;
  const second = screen.getByText("second").parentElement!;
  const unpinned = screen.getByText("unpinned").parentElement!;
  fireEvent.mouseDown(first, { button: 0, clientX: 0, clientY: 0 });
  fireEvent.mouseMove(second, { clientX: 0, clientY: 2 });
  fireEvent.mouseUp(second);
  expect(onPinnedTasksReordered).not.toHaveBeenCalled();
  drag(first, second);
  await waitFor(() => expect(onPinnedTasksReordered).toHaveBeenCalledWith("first", "second"));
  drag(second, unpinned);
  expect(onPinnedTasksReordered).toHaveBeenCalledOnce();
});

it("shows persistence errors from reordering", async () => {
  const { onPinnedTasksReordered } = fixture();
  onPinnedTasksReordered.mockRejectedValue(new Error("Could not save changes"));
  drag(screen.getByText("first").parentElement!, screen.getByText("second").parentElement!);
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

it("renders every card of a long list and focuses a revealed one past the first rows", async () => {
  const tasks = Array.from({ length: 40 }, (_, index) => ({
    ...TASKS[2],
    id: `task-${index}`,
    branchName: `task-${index}`,
  }));
  fixture({ tasks, revealTaskId: "task-39" });
  expect(screen.getByText("task-0")).toBeInTheDocument();
  expect(screen.getByText("task-39")).toBeInTheDocument();
  await waitFor(() => expect(screen.getByText("task-39")).toHaveFocus());
});

it("scrolls a revealed card back into view when rows above it change height", async () => {
  const rects = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect");
  onTestFinished(() => rects.mockRestore());
  rects.mockImplementation(function (this: HTMLElement) {
    const scrollTop = this.closest(".overflow-y-auto")?.scrollTop ?? 0;
    const rowsAboveGrowth = scrollTop > 0 ? 300 : 0;
    const top = this.dataset.selected ? 1000 + rowsAboveGrowth - scrollTop : 0;
    return { top, bottom: top + (this.dataset.selected ? 100 : 400) } as DOMRect;
  });
  fixture({ revealTaskId: "unpinned" });
  const container = screen.getByText("unpinned").closest<HTMLElement>(".overflow-y-auto")!;
  await waitFor(() => expect(container.scrollTop).toBe(1276));
  expect(screen.getByText("unpinned")).toHaveFocus();
});

function deferredDeletion() {
  const deferred = {} as {
    resolve: (value: { data: { id: string }; warnings: [] }) => void;
    reject: (error: Error) => void;
  };
  const onTaskDeleted = vi.fn<TaskActions["delete"]>(
    () => new Promise((resolve, reject) => Object.assign(deferred, { resolve, reject }))
  );
  return { deferred, onTaskDeleted };
}

it("dims a deleting card and restores it without a toast when deletion fails", async () => {
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
  expect(screen.queryByText("git worktree remove failed")).not.toBeInTheDocument();
});

it("keeps a deleting card in its own workspace and reports its failure from another", async () => {
  const { deferred, onTaskDeleted } = deferredDeletion();
  const { switchWorkspace } = fixture({ onTaskDeleted });
  fireEvent.click(screen.getByText("delete second"));
  switchWorkspace("w2", []);
  expect(screen.queryByText("second")).not.toBeInTheDocument();
  expect(screen.getByText("No active tasks")).toBeInTheDocument();
  await act(async () => deferred.reject(new Error("git worktree remove failed")));
  expect(
    screen.getByText('Could not delete "second". Open its workspace to review.')
  ).toBeInTheDocument();
  switchWorkspace("w1", TASKS);
  expect(screen.getByText("second").parentElement).not.toHaveAttribute("inert");
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

it("keeps shortcut help visible when the workspace has no tasks", () => {
  const { setTasks } = fixture();
  setTasks([]);
  expect(screen.getByText("No active tasks")).toBeInTheDocument();
  expect(screen.getByText(/palette/)).toBeInTheDocument();
  expect(screen.getByText(/history/)).toBeInTheDocument();
});

const AUTOMATION_TASKS = TASKS.map((task, index) =>
  index === 0 ? { ...task, automationId: "pr-babysitter" } : task
);
const listedTaskIds = () =>
  vi.mocked(useWorktreeListKeyboardShortcuts).mock.lastCall?.[0].tasks.map((task) => task.id);
const automationToggle = () => screen.getByRole("checkbox", { name: "Automation tasks (1)" });

it("shows automation tasks when no preference was saved", async () => {
  fixture({ tasks: AUTOMATION_TASKS });
  await waitFor(() => expect(loadShowAutomationTasks).toHaveBeenCalled());
  expect(screen.getByText("first")).toBeInTheDocument();
  expect(listedTaskIds()).toEqual(["first", "second", "unpinned"]);
  expect(automationToggle()).toBeChecked();
});

it("hides automation tasks when the saved preference hides them", async () => {
  vi.mocked(loadShowAutomationTasks).mockResolvedValue(false);
  fixture({ tasks: AUTOMATION_TASKS });
  await waitFor(() => expect(screen.queryByText("first")).not.toBeInTheDocument());
  expect(listedTaskIds()).toEqual(["second", "unpinned"]);
  expect(automationToggle()).not.toBeChecked();
});

it("remembers the toggle choice", () => {
  fixture({ tasks: AUTOMATION_TASKS });
  fireEvent.click(automationToggle());
  expect(screen.queryByText("first")).not.toBeInTheDocument();
  fireEvent.click(automationToggle());
  expect(screen.getByText("first")).toBeInTheDocument();
  expect(vi.mocked(persist).mock.calls).toEqual([
    [[["showAutomationTasks", false]]],
    [[["showAutomationTasks", true]]],
  ]);
});

it("keeps a choice made before the saved preference loads", async () => {
  let resolveSaved: (show: boolean) => void = () => undefined;
  vi.mocked(loadShowAutomationTasks).mockReturnValue(new Promise((r) => (resolveSaved = r)));
  fixture({ tasks: AUTOMATION_TASKS });
  fireEvent.click(automationToggle());
  await act(async () => resolveSaved(true));
  expect(screen.queryByText("first")).not.toBeInTheDocument();
});

it("keeps showing automation tasks when the preference cannot be read", async () => {
  vi.mocked(loadShowAutomationTasks).mockRejectedValue(new Error("store unavailable"));
  fixture({ tasks: AUTOMATION_TASKS });
  await waitFor(() => expect(loadShowAutomationTasks).toHaveBeenCalled());
  expect(screen.getByText("first")).toBeInTheDocument();
});

it("keeps the choice for the session when it cannot be saved", async () => {
  vi.mocked(persist).mockRejectedValue(new Error("disk full"));
  fixture({ tasks: AUTOMATION_TASKS });
  fireEvent.click(automationToggle());
  await waitFor(() => expect(persist).toHaveBeenCalled());
  expect(screen.queryByText("first")).not.toBeInTheDocument();
});

it("reveals a hidden automation task without changing the saved preference", async () => {
  let resolveSaved: (show: boolean) => void = () => undefined;
  vi.mocked(loadShowAutomationTasks).mockReturnValue(new Promise((r) => (resolveSaved = r)));
  const { setTasks } = fixture({ revealTaskId: "first" });
  await act(async () => resolveSaved(false));
  setTasks(AUTOMATION_TASKS);
  expect(screen.getByText("first")).toBeInTheDocument();
  expect(automationToggle()).toBeChecked();
  expect(persist).not.toHaveBeenCalled();
});

it("shows a hidden automation task when returning from it to the list", async () => {
  vi.mocked(loadShowAutomationTasks).mockResolvedValue(false);
  fixture({
    tasks: AUTOMATION_TASKS,
    openedTask: { taskId: "first", surface: { kind: "external" } },
  });
  await waitFor(() => expect(screen.queryByText("first")).not.toBeInTheDocument());
  fireEvent.click(screen.getByRole("button", { name: "Back" }));
  expect(screen.getByText("first")).toBeInTheDocument();
  expect(persist).not.toHaveBeenCalled();
});

const SHELVED_AT = "2026-02-01T00:00:00.000Z";

function withFinished(...ids: string[]) {
  return TASKS.map((task) => (ids.includes(task.id) ? { ...task, shelvedAt: SHELVED_AT } : task));
}

function finishedSection({ hidden = false } = {}) {
  return screen.getByRole("button", { name: /Finished/, hidden });
}

function keyboardTaskIds() {
  return vi
    .mocked(useWorktreeListKeyboardShortcuts)
    .mock.lastCall?.[0].tasks.map((task) => task.id);
}

it("keeps finished tasks in a collapsed section out of keyboard navigation", () => {
  fixture({ tasks: withFinished("second") });
  expect(finishedSection()).toHaveAttribute("aria-expanded", "false");
  expect(finishedSection()).toHaveTextContent("1");
  expect(screen.queryByText("second")).not.toBeInTheDocument();
  expect(keyboardTaskIds()).toEqual(["first", "unpinned"]);
});

it("lists finished tasks after the active ones once the section is expanded", () => {
  fixture({ tasks: withFinished("first") });
  fireEvent.click(finishedSection());
  expect(screen.getByText("first")).toBeInTheDocument();
  expect(screen.getByText("first").parentElement).not.toHaveAttribute("data-reorder-key");
  expect(keyboardTaskIds()).toEqual(["second", "unpinned", "first"]);
});

it("shows the Finished section when every task is finished", () => {
  fixture({ tasks: withFinished("first", "second", "unpinned") });
  expect(screen.queryByText("No tasks to show")).not.toBeInTheDocument();
  expect(finishedSection()).toHaveTextContent("3");
});

it("expands the Finished section to reveal a finished task", () => {
  fixture({ tasks: withFinished("second"), revealTaskId: "second" });
  expect(finishedSection()).toHaveAttribute("aria-expanded", "true");
  expect(screen.getByText("second")).toHaveAttribute("data-selected", "true");
});

it("expands the Finished section when returning from a finished task", () => {
  fixture({
    tasks: withFinished("second"),
    openedTask: { taskId: "second", surface: { kind: "external" } },
  });
  fireEvent.click(screen.getByRole("button", { name: "Back" }));
  expect(finishedSection({ hidden: true })).toHaveAttribute("aria-expanded", "true");
  expect(screen.getByText("second")).toBeInTheDocument();
});

it("moves tasks to Finished and back", async () => {
  const { onTaskShelved } = fixture({ tasks: withFinished("second") });
  fireEvent.click(screen.getByText("shelve first"));
  fireEvent.click(finishedSection());
  fireEvent.click(screen.getByText("shelve second"));
  await waitFor(() =>
    expect(onTaskShelved.mock.calls).toEqual([
      ["first", true],
      ["second", false],
    ])
  );
});

it("shows persistence errors from moving a task to Finished", async () => {
  const { onTaskShelved } = fixture();
  onTaskShelved.mockRejectedValue(new Error("Could not save changes"));
  fireEvent.click(screen.getByText("shelve first"));
  expect(await screen.findByText("Error: Could not save changes")).toBeInTheDocument();
});

it("keeps the Finished section for the last finished card's exit", async () => {
  Element.prototype.animate = vi.fn(
    () => ({ cancel: vi.fn(), onfinish: null }) as unknown as Animation
  );
  try {
    const { deferred, onTaskDeleted } = deferredDeletion();
    const { setTasks } = fixture({ tasks: withFinished("first").slice(0, 1), onTaskDeleted });
    fireEvent.click(finishedSection());
    fireEvent.click(screen.getByText("delete first"));
    setTasks([]);
    await act(async () => deferred.resolve({ data: { id: "first" }, warnings: [] }));
    expect(screen.getByText("first").closest("[inert]")).not.toBeNull();
  } finally {
    delete (Element.prototype as Partial<Element>).animate;
  }
});

function withSnoozed(until: string, ...ids: string[]) {
  return TASKS.map((task) => (ids.includes(task.id) ? { ...task, snoozedUntil: until } : task));
}

function section(name: "Snoozed" | "Finished") {
  return screen.getByRole("button", { name: new RegExp(name) });
}

it("keeps snoozed tasks in their own collapsed section until the snooze ends", () => {
  fixture({ tasks: withSnoozed(LATER, "first") });
  expect(section("Snoozed")).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByText("first")).not.toBeInTheDocument();
  expect(keyboardTaskIds()).toEqual(["second", "unpinned"]);
});

it("lists a task whose snooze ended among the active ones", () => {
  fixture({ tasks: withSnoozed("2000-01-01T00:00:00.000Z", "first") });
  expect(screen.queryByRole("button", { name: /Snoozed/ })).not.toBeInTheDocument();
  expect(keyboardTaskIds()).toEqual(["first", "second", "unpinned"]);
});

it("walks active, snoozed and finished tasks in that order once expanded", () => {
  fixture({
    tasks: withSnoozed(LATER, "first").map((task) =>
      task.id === "second" ? { ...task, shelvedAt: SHELVED_AT } : task
    ),
  });
  fireEvent.click(section("Finished"));
  fireEvent.click(section("Snoozed"));
  expect(keyboardTaskIds()).toEqual(["unpinned", "first", "second"]);
});

it("expands the Snoozed section to reveal a snoozed task", () => {
  fixture({ tasks: withSnoozed(LATER, "second"), revealTaskId: "second" });
  expect(section("Snoozed")).toHaveAttribute("aria-expanded", "true");
  expect(screen.getByText("second")).toHaveAttribute("data-selected", "true");
});

it("snoozes a task from its card", async () => {
  const { onTaskSnoozed } = fixture();
  fireEvent.click(screen.getByText("snooze first"));
  await waitFor(() => expect(onTaskSnoozed).toHaveBeenCalledWith("first", LATER));
});

it("shows persistence errors from snoozing a task", async () => {
  const { onTaskSnoozed } = fixture();
  onTaskSnoozed.mockRejectedValue(new Error("Could not save changes"));
  fireEvent.click(screen.getByText("snooze first"));
  expect(await screen.findByText("Error: Could not save changes")).toBeInTheDocument();
});

it("snoozes a task until the date picked in the dialog", async () => {
  const { onTaskSnoozed } = fixture();
  fireEvent.click(screen.getByText("pick snooze first"));
  fireEvent.change(screen.getByLabelText("Snooze until"), { target: { value: "2099-03-04" } });
  fireEvent.click(screen.getByRole("button", { name: "Snooze" }));
  await waitFor(() =>
    expect(onTaskSnoozed).toHaveBeenCalledWith("first", new Date(2099, 2, 4).toISOString())
  );
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
});
