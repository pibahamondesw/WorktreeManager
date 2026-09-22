// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { NewWorktreeModal } from "./NewWorktreeModal";
import { CreateTaskInput, OperationError, TaskReady } from "../../services/operations";
import { Task, Workspace } from "../../types";

vi.mock("../../contexts/useLinear", () => ({ useLinear: () => null }));
vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion: vi.fn() }));
vi.mock("../../services/store", () => ({ persist: vi.fn() }));
vi.mock("../../services/taskSessions", () => ({ closeTaskSessions: vi.fn() }));
afterEach(cleanup);

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
  const branch = screen.getByPlaceholderText("feature/my-branch");
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
  fireEvent.change(screen.getByPlaceholderText("feature/my-branch"), {
    target: { value: "feature" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
  await waitFor(() =>
    expect(screen.getByText(/Created: API/)).toHaveTextContent("Failed: Web · /web/feature")
  );
});
