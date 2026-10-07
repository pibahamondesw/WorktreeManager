// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { NewWorktreeModal } from "./NewWorktreeModal";
import { CreateTaskInput, OperationError, TaskReady } from "../../services/operations";
import { Task, Workspace } from "../../types";

const linearMock = vi.hoisted(() => ({
  current: null as null | Record<string, ReturnType<typeof vi.fn>>,
}));
vi.mock("../../contexts/useLinear", () => ({ useLinear: () => linearMock.current }));
vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion: vi.fn() }));
vi.mock("../../services/store", () => ({ persist: vi.fn() }));
vi.mock("../../services/taskSessions", () => ({ closeTaskSessions: vi.fn() }));
afterEach(() => {
  cleanup();
  linearMock.current = null;
});

const workspace: Workspace = {
  id: "workspace",
  name: "Workspace",
  repos: [{ id: "api", name: "API", localPath: "/api", worktreeBasePath: "/worktrees" }],
};
const task: Task = {
  id: "task",
  workspaceId: workspace.id,
  branchName: "feature",
  createdAt: "2026-09-22",
  members: [
    {
      repoId: "api",
      repoName: "API",
      localPath: "/api",
      path: "/worktrees/feature",
      branchName: "feature",
    },
  ],
};

it("opens and closes on readiness while setup remains pending, without duplicate submissions", async () => {
  let notifyReady!: TaskReady;
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const onCreated = vi.fn(
    async (_input: CreateTaskInput, _progress?: (message: string) => void, ready?: TaskReady) => {
      notifyReady = ready!;
      await pending;
      return { data: task, warnings: [] };
    }
  );
  const onClose = vi.fn();
  const onOpenTask = vi.fn().mockResolvedValue(true);
  render(
    <NewWorktreeModal
      open
      workspace={workspace}
      editorApp="cursor"
      onCreated={onCreated}
      onClose={onClose}
      onOpenTask={onOpenTask}
    />
  );
  const branch = screen.getByRole("textbox", { name: "Branch name" });
  fireEvent.change(branch, { target: { value: "feature" } });
  fireEvent.keyDown(branch, { key: "Enter" });
  fireEvent.keyDown(branch, { key: "Enter" });
  fireEvent.keyDown(document, { key: "Escape" });
  expect(onCreated).toHaveBeenCalledOnce();
  expect(onClose).not.toHaveBeenCalled();
  await act(async () => {
    await notifyReady(task);
  });
  expect(onOpenTask).toHaveBeenCalledWith(task, expect.any(Object));
  expect(onClose).toHaveBeenCalledOnce();
  await act(async () => {
    finish();
  });
  expect(onClose).toHaveBeenCalledOnce();
});

it("shows completed and failed destinations for partial creation", async () => {
  const onCreated = vi.fn().mockRejectedValue(
    new OperationError("create_failed", "Some worktrees could not be created.", {
      completedMembers: task.members,
      failedMembers: [{ ...task.members[0], repoName: "Web", path: "/web/feature" }],
    })
  );
  render(
    <NewWorktreeModal
      open
      workspace={workspace}
      editorApp="cursor"
      onCreated={onCreated}
      onClose={vi.fn()}
    />
  );
  fireEvent.change(screen.getByRole("textbox", { name: "Branch name" }), {
    target: { value: "feature" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
  await waitFor(() =>
    expect(screen.getByText(/Created: API/)).toHaveTextContent("Failed: Web · /web/feature")
  );
});

it("creates the Linear issue first and uses its branch name for the task", async () => {
  const order: string[] = [];
  const createdIssue = {
    id: "issue-1",
    identifier: "WOR-1",
    title: "New thing",
    branchName: "me/wor-1-new-thing",
    priority: 0,
    updatedAt: "2026-10-04",
  };
  linearMock.current = {
    fetchAssignedIssues: vi.fn().mockResolvedValue([]),
    listTeams: vi.fn().mockResolvedValue([{ id: "team-1", key: "WOR", name: "Worktree" }]),
    createIssue: vi.fn(async () => {
      order.push("issue");
      return createdIssue;
    }),
    startIssue: vi.fn().mockResolvedValue(undefined),
  };
  const onCreated = vi.fn(async (input: CreateTaskInput) => {
    order.push(`task:${input.branchName}`);
    return { data: task, warnings: [] };
  });
  render(
    <NewWorktreeModal
      open
      workspace={workspace}
      editorApp="cursor"
      onCreated={onCreated}
      onClose={vi.fn()}
    />
  );

  fireEvent.click(await screen.findByText("Create without Linear issue"));
  fireEvent.click(screen.getByLabelText("Also create Linear issue"));
  fireEvent.change(screen.getByPlaceholderText("What needs to be done?"), {
    target: { value: "New thing" },
  });
  await waitFor(() => expect(screen.getByRole("combobox")).toHaveValue("team-1"));
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));

  await waitFor(() => expect(onCreated).toHaveBeenCalled());
  expect(linearMock.current.createIssue).toHaveBeenCalledWith({
    teamId: "team-1",
    title: "New thing",
    description: "",
  });
  expect(order).toEqual(["issue", "task:me/wor-1-new-thing"]);
  expect(onCreated.mock.calls[0][0]).toMatchObject({
    branchName: "me/wor-1-new-thing",
    linearIssue: { id: "issue-1", identifier: "WOR-1" },
  });
});

async function renderNewIssueForm(
  onCreated: NewWorktreeModalPropsForTest["onCreated"],
  repos = workspace.repos
) {
  const service = {
    fetchAssignedIssues: vi.fn().mockResolvedValue([]),
    listTeams: vi.fn().mockResolvedValue([{ id: "team-1", key: "WOR", name: "Worktree" }]),
    createIssue: vi.fn().mockResolvedValue({
      id: "issue-1",
      identifier: "WOR-1",
      title: "New thing",
      branchName: "me/wor-1-new-thing",
      priority: 0,
      updatedAt: "2026-10-04",
    }),
    startIssue: vi.fn().mockResolvedValue(undefined),
  };
  linearMock.current = service;
  render(
    <NewWorktreeModal
      open
      workspace={{ ...workspace, repos }}
      editorApp="cursor"
      onCreated={onCreated}
      onClose={vi.fn()}
    />
  );
  fireEvent.click(await screen.findByText("Create without Linear issue"));
  fireEvent.click(screen.getByLabelText("Also create Linear issue"));
  fireEvent.change(screen.getByPlaceholderText("What needs to be done?"), {
    target: { value: "New thing" },
  });
  await waitFor(() => expect(screen.getByRole("combobox")).toHaveValue("team-1"));
  return service;
}

type NewWorktreeModalPropsForTest = React.ComponentProps<typeof NewWorktreeModal>;

it("reuses a created issue after worktree failure and locks its fields", async () => {
  const onCreated = vi
    .fn()
    .mockRejectedValueOnce(new Error("worktree failed"))
    .mockResolvedValue({ data: task, warnings: [] });
  const service = await renderNewIssueForm(onCreated);
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
  await screen.findByText("worktree failed");
  expect(screen.getByLabelText("Issue title")).toBeDisabled();
  expect(screen.getByLabelText("Team")).toBeDisabled();
  expect(screen.getByLabelText("Description (optional)")).toBeDisabled();
  expect(screen.getByText(/Issue WOR-1 was created/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
  await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(2));
  expect(service.createIssue).toHaveBeenCalledOnce();
  expect(onCreated.mock.calls[1][0]).toEqual(onCreated.mock.calls[0][0]);
});

it("uses the manual branch after disabling issue creation and retains the issue for a later retry", async () => {
  const onCreated = vi.fn().mockRejectedValue(new Error("worktree failed"));
  const service = await renderNewIssueForm(onCreated);
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
  await screen.findByText("worktree failed");
  fireEvent.click(screen.getByLabelText("Also create Linear issue"));
  fireEvent.change(screen.getByRole("textbox", { name: "Branch name" }), {
    target: { value: "manual-retry" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
  await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(2));
  expect(onCreated.mock.calls[1][0]).toEqual({
    workspaceId: "workspace",
    branchName: "manual-retry",
    repoIds: ["api"],
  });
  await screen.findByText("worktree failed");
  fireEvent.click(screen.getByLabelText("Also create Linear issue"));
  expect(screen.getByLabelText("Issue title")).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
  await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(3));
  expect(service.createIssue).toHaveBeenCalledOnce();
  expect(onCreated.mock.calls[2][0]).toMatchObject({ linearIssue: { id: "issue-1" } });
});

it("does not create worktrees when Linear rejects creation and allows correcting the draft", async () => {
  const onCreated = vi.fn().mockResolvedValue({ data: task, warnings: [] });
  const service = await renderNewIssueForm(onCreated);
  service.createIssue.mockRejectedValueOnce(new Error("Linear unavailable"));
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
  await screen.findByText("Linear unavailable");
  expect(onCreated).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Issue title")).toBeEnabled();
  fireEvent.change(screen.getByLabelText("Issue title"), { target: { value: "Corrected title" } });
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
  await waitFor(() => expect(onCreated).toHaveBeenCalledOnce());
  expect(service.createIssue).toHaveBeenLastCalledWith({
    teamId: "team-1",
    title: "Corrected title",
    description: "",
  });
});

it("validates repository selection before creating an issue and passes only selected repositories", async () => {
  const onCreated = vi.fn().mockResolvedValue({ data: task, warnings: [] });
  const service = await renderNewIssueForm(onCreated, [
    ...workspace.repos,
    { id: "web", name: "Web", localPath: "/web", worktreeBasePath: "/webtrees" },
  ]);
  fireEvent.click(screen.getByLabelText("API"));
  fireEvent.click(screen.getByLabelText("Web"));
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
  await screen.findByText("Select at least one repository");
  expect(service.createIssue).not.toHaveBeenCalled();
  fireEvent.click(screen.getByLabelText("Web"));
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
  await waitFor(() => expect(onCreated).toHaveBeenCalledOnce());
  expect(onCreated.mock.calls[0][0]).toMatchObject({ repoIds: ["web"] });
});

it("prevents duplicate submissions and mode changes while issue creation is pending", async () => {
  const onCreated = vi.fn().mockResolvedValue({ data: task, warnings: [] });
  const service = await renderNewIssueForm(onCreated);
  let finish!: (issue: Awaited<ReturnType<typeof service.createIssue>>) => void;
  const pending = new Promise((resolve) => {
    finish = resolve;
  });
  service.createIssue.mockReturnValueOnce(pending);
  const title = screen.getByLabelText("Issue title");
  fireEvent.keyDown(title, { key: "Enter" });
  fireEvent.keyDown(title, { key: "Enter" });
  expect(service.createIssue).toHaveBeenCalledOnce();
  expect(onCreated).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Also create Linear issue")).toBeDisabled();
  expect(screen.getByRole("button", { name: "Back to Linear issues" })).toBeDisabled();
  await act(async () => {
    finish({
      id: "issue-1",
      identifier: "WOR-1",
      title: "New thing",
      branchName: "me/wor-1-new-thing",
      priority: 0,
      updatedAt: "2026-10-04",
    });
  });
  expect(onCreated).toHaveBeenCalledOnce();
});

it("shows team loading errors and keeps issue creation disabled", async () => {
  const onCreated = vi.fn();
  linearMock.current = {
    fetchAssignedIssues: vi.fn().mockResolvedValue([]),
    listTeams: vi.fn().mockRejectedValue(new Error("offline")),
    createIssue: vi.fn(),
  };
  render(
    <NewWorktreeModal
      open
      workspace={workspace}
      editorApp="cursor"
      onCreated={onCreated}
      onClose={vi.fn()}
    />
  );
  fireEvent.click(await screen.findByText("Create without Linear issue"));
  fireEvent.click(screen.getByLabelText("Also create Linear issue"));
  fireEvent.change(screen.getByLabelText("Issue title"), { target: { value: "New thing" } });
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not load Linear teams");
  expect(screen.getByRole("button", { name: "Create Task" })).toBeDisabled();
  expect(linearMock.current.createIssue).not.toHaveBeenCalled();
  expect(onCreated).not.toHaveBeenCalled();
});
