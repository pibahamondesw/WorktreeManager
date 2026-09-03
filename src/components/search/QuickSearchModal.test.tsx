// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { QuickSearchModal } from "./QuickSearchModal";
import { Task, Workspace } from "../../types";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("../../services/openEditor", () => ({
  openEditorForWorktree: vi.fn().mockResolvedValue(true),
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
  members: [],
  createdAt: "2026-01-01T00:00:00Z",
};

function renderModal() {
  const onReveal = vi.fn();
  render(
    <QuickSearchModal
      open
      onClose={vi.fn()}
      initialQuery="ledger-sync"
      tasks={[otherTask]}
      workspaces={workspaces}
      selectedWorkspaceId="ws-1"
      editorApp="cursor"
      onReveal={onReveal}
    />
  );
  return onReveal;
}

afterEach(cleanup);

describe("QuickSearchModal navigation", () => {
  it("reveals the task in its workspace when opened with Enter", () => {
    const onReveal = renderModal();
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });
    expect(onReveal).toHaveBeenCalledWith(otherTask);
  });

  it("reveals the task in its workspace when the result is clicked", () => {
    const onReveal = renderModal();
    fireEvent.click(screen.getByRole("button", { name: /feature\/ledger-sync/ }));
    expect(onReveal).toHaveBeenCalledWith(otherTask);
  });
});
