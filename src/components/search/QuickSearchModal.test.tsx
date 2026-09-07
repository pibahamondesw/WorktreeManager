// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { QuickSearchModal } from "./QuickSearchModal";
import { Task, Workspace } from "../../types";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

Element.prototype.scrollIntoView = vi.fn();

const workspaces: Workspace[] = [
  { id: "ws-1", name: "Payments", repos: [] },
  { id: "ws-2", name: "Ledger", repos: [] },
];

const otherTask: Task = {
  id: "t-2",
  workspaceId: "ws-2",
  branchName: "feature/ledger-sync",
  members: [],
  createdAt: "2026-01-01T00:00:00Z",
};

function renderModal(props: Partial<React.ComponentProps<typeof QuickSearchModal>> = {}) {
  const onReveal = vi.fn();
  const onOpenTask = vi.fn().mockResolvedValue(true);
  render(
    <QuickSearchModal
      open
      onClose={vi.fn()}
      initialQuery="ledger-sync"
      tasks={[otherTask]}
      workspaces={workspaces}
      selectedWorkspaceId="ws-1"
      onReveal={onReveal}
      onOpenTask={onOpenTask}
      {...props}
    />
  );
  return { onReveal, onOpenTask };
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

  it("opens the task when the result is clicked", () => {
    const { onOpenTask } = renderModal();
    fireEvent.click(screen.getByRole("button", { name: /feature\/ledger-sync/ }));
    expect(onOpenTask).toHaveBeenCalledWith(otherTask, expect.any(Object));
  });
});

describe("QuickSearchModal ordering", () => {
  it("lists the most recently visited task first", () => {
    const older: Task = { ...otherTask, id: "t-old", branchName: "feature/old", createdAt: "2026-01-01T00:00:00Z" };
    const newer: Task = { ...otherTask, id: "t-new", branchName: "feature/new", createdAt: "2026-05-01T00:00:00Z" };
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
});
