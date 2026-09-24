// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { QuickSearchModal } from "./QuickSearchModal";
import { Task, Workspace } from "../../types";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("../../services/pullRequest", () => ({
  openPrForMember: vi.fn().mockResolvedValue(true),
}));

Element.prototype.scrollIntoView = vi.fn();

const workspaces: Workspace[] = [
  { id: "ws-1", name: "Payments", repos: [] },
  { id: "ws-2", name: "Ledger", repos: [] },
];

const otherTask: Task = {
  id: "t-2",
  workspaceId: "ws-2",
  branchName: "feature/ledger-sync",
  linearIssueIdentifier: "WOR-12",
  members: [
    {
      repoId: "r1",
      repoName: "ledger",
      localPath: "/repos/ledger",
      path: "/wt/ledger",
      branchName: "feature/ledger-sync",
    },
  ],
  createdAt: "2026-01-01T00:00:00Z",
};

function renderModal(props: Partial<React.ComponentProps<typeof QuickSearchModal>> = {}) {
  const onReveal = vi.fn();
  const onOpenTask = vi.fn().mockResolvedValue(true);
  const onSelectWorkspace = vi.fn();
  const onWorkspaceAction = vi.fn();
  const onThemeChange = vi.fn();
  const onEditorChange = vi.fn();
  const onNewTask = vi.fn();
  render(
    <QuickSearchModal
      open
      onClose={vi.fn()}
      initialQuery="ledger-sync"
      tasks={[otherTask]}
      workspaces={workspaces}
      selectedWorkspaceId="ws-1"
      themeId="default"
      editorApp="cursor"
      onReveal={onReveal}
      onOpenTask={onOpenTask}
      onSelectWorkspace={onSelectWorkspace}
      onWorkspaceAction={onWorkspaceAction}
      onThemeChange={onThemeChange}
      onEditorChange={onEditorChange}
      onNewTask={onNewTask}
      {...props}
    />
  );
  return {
    onReveal,
    onOpenTask,
    onSelectWorkspace,
    onWorkspaceAction,
    onThemeChange,
    onEditorChange,
    onNewTask,
  };
}

afterEach(cleanup);

describe("QuickSearchModal navigation", () => {
  it("opens the task when Enter is pressed", () => {
    const { onOpenTask } = renderModal();
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });
    expect(onOpenTask).toHaveBeenCalledWith(otherTask, expect.any(Object));
  });

  it("only reveals the task when Enter is pressed with meta", () => {
    const { onReveal, onOpenTask } = renderModal();
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter", metaKey: true });
    expect(onReveal).toHaveBeenCalledWith(otherTask);
    expect(onOpenTask).not.toHaveBeenCalled();
  });

  it("opens the first task on a blank query even when commands are listed", () => {
    const { onOpenTask } = renderModal({ initialQuery: "" });
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });
    expect(onOpenTask).toHaveBeenCalledWith(otherTask, expect.any(Object));
  });

  it("opens the task when the result is clicked", () => {
    const { onOpenTask } = renderModal();
    fireEvent.click(screen.getByRole("button", { name: /feature\/ledger-sync/ }));
    expect(onOpenTask).toHaveBeenCalledWith(otherTask, expect.any(Object));
  });

  it("navigates results with the arrow keys", () => {
    const secondTask = { ...otherTask, id: "t-3", branchName: "feature/second" };
    const { onOpenTask } = renderModal({ initialQuery: "", tasks: [otherTask, secondTask] });
    const input = screen.getByRole("textbox");

    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(onOpenTask).toHaveBeenCalledWith(secondTask, expect.any(Object));
  });

  it("jumps to a task result by number", () => {
    const secondTask = { ...otherTask, id: "t-3", branchName: "feature/second" };
    const { onOpenTask } = renderModal({ initialQuery: "", tasks: [otherTask, secondTask] });
    const input = screen.getByRole("textbox");

    fireEvent.keyDown(input, { key: "1" });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(onOpenTask).toHaveBeenCalledWith(secondTask, expect.any(Object));
  });

  it("keeps number keys available while searching", () => {
    renderModal({ initialQuery: "wor-" });

    expect(fireEvent.keyDown(screen.getByRole("textbox"), { key: "1" })).toBe(true);
  });
});

describe("QuickSearchModal ordering", () => {
  it("lists the most recently visited task first", () => {
    const older: Task = {
      ...otherTask,
      id: "t-old",
      branchName: "feature/old",
      createdAt: "2026-01-01T00:00:00Z",
    };
    const newer: Task = {
      ...otherTask,
      id: "t-new",
      branchName: "feature/new",
      createdAt: "2026-05-01T00:00:00Z",
    };
    renderModal({
      initialQuery: "",
      tasks: [newer, older],
      historyEntries: [
        { kind: "task", taskId: "t-old", workspaceId: "ws-2", at: "2026-09-01T00:00:00Z" },
      ],
    });
    const names = screen.getAllByRole("button", { name: /feature\// }).map((b) => b.textContent);
    expect(names[0]).toContain("feature/old");
    expect(names[1]).toContain("feature/new");
  });

  it("lists an active session from another workspace first", () => {
    const currentTask: Task = {
      ...otherTask,
      id: "t-current",
      workspaceId: "ws-1",
      branchName: "feature/current",
    };
    renderModal({
      initialQuery: "",
      tasks: [currentTask, otherTask],
      agentSessions: { "t-2": { kind: "running" } },
    });
    const names = screen
      .getAllByRole("button", { name: /feature\// })
      .map((button) => button.textContent);
    expect(names[0]).toContain("feature/ledger-sync");
    expect(screen.getByTitle("Agent session active")).toBeInTheDocument();
  });

  it("filters to active sessions from the filter tab", () => {
    const currentTask: Task = {
      ...otherTask,
      id: "t-current",
      workspaceId: "ws-1",
      branchName: "feature/current",
    };
    renderModal({
      initialQuery: "",
      tasks: [currentTask, otherTask],
      agentSessions: { "t-2": { kind: "running" } },
    });

    fireEvent.click(screen.getByRole("button", { name: "Active sessions" }));

    expect(screen.getByRole("textbox")).toHaveValue("session:active ");
    expect(screen.getByRole("button", { name: /feature\/ledger-sync/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /feature\/current/ })).not.toBeInTheDocument();
  });
});

describe("QuickSearchModal commands", () => {
  it("runs a workspace command from the palette", () => {
    const { onSelectWorkspace } = renderModal({ initialQuery: ">switch ledger" });
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });
    expect(onSelectWorkspace).toHaveBeenCalledWith("ws-2");
  });

  it("opens vault settings from a matching command", () => {
    const { onWorkspaceAction } = renderModal({ initialQuery: "obsidian" });
    fireEvent.click(screen.getByRole("button", { name: /Obsidian vault/ }));
    expect(onWorkspaceAction).toHaveBeenCalledWith({ kind: "vault" });
  });

  it("opens Claude Code for the matching task", () => {
    const { onOpenTask } = renderModal({ initialQuery: "claude wor-12" });
    fireEvent.click(screen.getByRole("button", { name: /Open in Claude Code/ }));
    expect(onOpenTask).toHaveBeenCalledWith(
      otherTask,
      expect.objectContaining({ agent: "claude" })
    );
  });

  it("copies the Linear identifier without closing", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    const onClose = vi.fn();
    renderModal({ initialQuery: "copy linear wor-12", onClose });
    fireEvent.click(screen.getByRole("button", { name: /Copy Linear ID/ }));
    expect(writeText).toHaveBeenCalledWith("WOR-12");
    expect(onClose).not.toHaveBeenCalled();
    expect(await screen.findByText("Linear ID copied")).toBeInTheDocument();
  });

  it("opens the new-task modal", () => {
    const { onNewTask } = renderModal({ initialQuery: ">new task" });
    fireEvent.click(screen.getByRole("button", { name: /New task/ }));
    expect(onNewTask).toHaveBeenCalledOnce();
  });

  it("runs a visible command from its shortcut", () => {
    const onClose = vi.fn();
    const { onNewTask } = renderModal({ initialQuery: "", onClose });

    fireEvent.keyDown(screen.getByRole("textbox"), { key: "n" });

    expect(onNewTask).toHaveBeenCalledOnce();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("runs a visible workspace command from its meta shortcut", () => {
    const { onSelectWorkspace } = renderModal({ initialQuery: "" });

    fireEvent.keyDown(screen.getByRole("textbox"), { key: "1", metaKey: true });

    expect(onSelectWorkspace).toHaveBeenCalledWith("ws-2");
  });
});
