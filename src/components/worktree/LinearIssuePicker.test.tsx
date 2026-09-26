// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { LinearIssuePicker } from "./LinearIssuePicker";
import { NewWorktreeModal } from "./NewWorktreeModal";
import { LinearIssue, Workspace, Task } from "../../types";
import { CreateTaskInput, TaskReady } from "../../services/operations";

const { fetchAssignedIssues, startIssue } = vi.hoisted(() => ({
  fetchAssignedIssues: vi.fn(),
  startIssue: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../../contexts/useLinear", () => {
  const service = { fetchAssignedIssues, startIssue };
  return { useLinear: () => service };
});
vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion: vi.fn() }));
vi.mock("../../services/openEditor", () => ({ openEditorForWorktree: vi.fn() }));
Element.prototype.scrollIntoView = vi.fn();

const assigned: LinearIssue = {
  id: "assigned",
  identifier: "WOR-123",
  title: "Link tasks",
  branchName: "user/link-tasks",
  projectName: "Workspace",
  description: "Existing tasks",
  priority: 2,
  stateName: "Todo",
  updatedAt: "2026-09-21",
};
const remote: LinearIssue = {
  ...assigned,
  id: "remote",
  identifier: "WOR-124",
  title: "Remote issue",
};
beforeEach(() => {
  vi.clearAllMocks();
  fetchAssignedIssues.mockResolvedValue([assigned]);
});
afterEach(cleanup);

const third: LinearIssue = { ...assigned, id: "third", identifier: "WOR-125", title: "Third" };

function searchInput() {
  return screen.getByRole("textbox", { name: "Search Linear issues" });
}

function search(value: string) {
  fireEvent.change(searchInput(), { target: { value } });
}

function press(key: string, init: Partial<KeyboardEventInit> = {}) {
  fireEvent.keyDown(searchInput(), { key, ...init });
}

function activeIssue() {
  return document.querySelector('[data-active="true"]');
}

describe("shared Linear issue picker", () => {
  it("filters assigned issues immediately and merges remote results without duplicates", async () => {
    const onSelect = vi.fn();
    render(<LinearIssuePicker onSelect={onSelect} />);
    expect(await screen.findByRole("button", { name: /WOR-123/ })).toHaveTextContent("High");
    fetchAssignedIssues.mockResolvedValue([assigned, remote]);
    search("Workspace");
    expect(screen.getByRole("button", { name: /WOR-123/ })).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: /WOR-124/ })).toBeInTheDocument();
    expect(fetchAssignedIssues).toHaveBeenLastCalledWith("Workspace");
    expect(screen.getAllByRole("button", { name: /WOR-123/ })).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: /WOR-124/ }));
    expect(onSelect).toHaveBeenCalledWith(remote);
  });

  it("navigates results with arrows, ctrl+n/p and digits, and selects with Enter", async () => {
    fetchAssignedIssues.mockResolvedValue([assigned, remote, third]);
    const onSelect = vi.fn();
    render(<LinearIssuePicker onSelect={onSelect} />);
    await screen.findByRole("button", { name: /WOR-125/ });
    expect(activeIssue()).toHaveTextContent("WOR-123");
    press("ArrowDown");
    press("ArrowDown");
    press("ArrowDown");
    expect(activeIssue()).toHaveTextContent("WOR-125");
    press("p", { ctrlKey: true });
    expect(activeIssue()).toHaveTextContent("WOR-124");
    press("ArrowUp");
    press("ArrowUp");
    expect(activeIssue()).toHaveTextContent("WOR-123");
    press("n", { ctrlKey: true });
    expect(activeIssue()).toHaveTextContent("WOR-124");
    press("2");
    expect(activeIssue()).toHaveTextContent("WOR-125");
    press("7");
    expect(activeIssue()).toHaveTextContent("WOR-125");
    press("Enter");
    expect(onSelect).toHaveBeenCalledWith(third);
  });

  it("keeps digits for typing once a query is entered", async () => {
    fetchAssignedIssues.mockResolvedValue([assigned, remote, third]);
    render(<LinearIssuePicker onSelect={vi.fn()} />);
    await screen.findByRole("button", { name: /WOR-125/ });
    search("WOR");
    const event = new KeyboardEvent("keydown", { key: "1", bubbles: true, cancelable: true });
    searchInput().dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
    expect(activeIssue()).toHaveTextContent("WOR-123");
  });

  it("ignores navigation keys while a selection is shown", async () => {
    const onSelect = vi.fn();
    render(
      <LinearIssuePicker onSelect={onSelect}>
        <p>Selected</p>
      </LinearIssuePicker>
    );
    press("Enter");
    press("ArrowDown");
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("ignores a late response after the query changes", async () => {
    render(<LinearIssuePicker onSelect={vi.fn()} />);
    await screen.findByRole("button", { name: /WOR-123/ });
    let resolve!: (issues: LinearIssue[]) => void;
    fetchAssignedIssues.mockImplementationOnce(
      () =>
        new Promise<LinearIssue[]>((done) => {
          resolve = done;
        })
    );
    search("old query");
    await waitFor(() => expect(fetchAssignedIssues).toHaveBeenCalledWith("old query"));
    search("");
    await act(async () => resolve([remote]));
    expect(screen.queryByRole("button", { name: /WOR-124/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /WOR-123/ })).toBeInTheDocument();
  });

  it("shows load and search failures and recovers with another query", async () => {
    fetchAssignedIssues.mockRejectedValueOnce(new Error("offline"));
    render(<LinearIssuePicker onSelect={vi.fn()} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Failed to load issues");
    fetchAssignedIssues.mockRejectedValueOnce(new Error("offline"));
    search("missing");
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("Failed to search issues")
    );
    fetchAssignedIssues.mockResolvedValue([remote]);
    search("remote");
    expect(await screen.findByRole("button", { name: /WOR-124/ })).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("still creates a task from the issue selected in New task", async () => {
    const onCreated = vi.fn(
      async (
        _input: CreateTaskInput,
        _progress?: (message: string) => void,
        onReady?: TaskReady
      ) => {
        const task = {
          id: "task",
          members: [],
          branchName: assigned.branchName,
        } as unknown as Task;
        await onReady?.(task);
        return { data: task, warnings: [] };
      }
    );
    const onClose = vi.fn();
    const workspace: Workspace = {
      id: "workspace",
      name: "Workspace",
      repos: [{ id: "repo", name: "Repo", localPath: "/repo", worktreeBasePath: "/worktrees" }],
    };
    render(
      <NewWorktreeModal
        open
        onClose={onClose}
        workspace={workspace}
        onCreated={onCreated}
        editorApp="cursor"
      />
    );
    fireEvent.click(await screen.findByRole("button", { name: /WOR-123/ }));
    fireEvent.click(screen.getByRole("button", { name: "Back to results" }));
    fireEvent.click(screen.getByRole("button", { name: /WOR-123/ }));
    fireEvent.click(screen.getByRole("button", { name: "Create Task" }));
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
    expect(onCreated).toHaveBeenCalledWith(
      {
        workspaceId: "workspace",
        branchName: assigned.branchName,
        repoIds: ["repo"],
        linearIssue: {
          id: assigned.id,
          identifier: assigned.identifier,
          title: assigned.title,
          projectName: assigned.projectName,
        },
      },
      expect.any(Function),
      expect.any(Function)
    );
  });
});
