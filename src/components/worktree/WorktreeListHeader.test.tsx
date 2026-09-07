// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { WorktreeListHeader } from "./WorktreeListHeader";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(false) }));

function renderHeader(
  sidebarCollapsed: boolean,
  history = { canGoBack: false, canGoForward: false }
) {
  const onExpandSidebar = vi.fn();
  const onGoBack = vi.fn();
  const onGoForward = vi.fn();
  render(
    <WorktreeListHeader
      workspaceName="Payments"
      taskCount={2}
      repoCount={1}
      editorApp="cursor"
      onEditorChange={vi.fn()}
      onRefresh={vi.fn()}
      refreshing={false}
      onNewTask={vi.fn()}
      onOpenSearch={vi.fn()}
      sidebarCollapsed={sidebarCollapsed}
      onExpandSidebar={onExpandSidebar}
      canGoBack={history.canGoBack}
      canGoForward={history.canGoForward}
      onGoBack={onGoBack}
      onGoForward={onGoForward}
    />
  );
  return { onExpandSidebar, onGoBack, onGoForward };
}

afterEach(cleanup);

describe("WorktreeListHeader sidebar toggle", () => {
  it("hides the expand button while the sidebar is visible", () => {
    renderHeader(false);
    expect(screen.queryByTitle("Expand sidebar [")).not.toBeInTheDocument();
  });

  it("expands the sidebar when the button is clicked", () => {
    const { onExpandSidebar } = renderHeader(true);
    fireEvent.click(screen.getByTitle("Expand sidebar ["));
    expect(onExpandSidebar).toHaveBeenCalledOnce();
  });
});

describe("WorktreeListHeader history controls", () => {
  it("disables both buttons when there is nowhere to go", () => {
    renderHeader(false);
    expect(screen.getByTitle("Back (⌘←)")).toBeDisabled();
    expect(screen.getByTitle("Forward (⌘→)")).toBeDisabled();
  });

  it("navigates when a direction is available", () => {
    const { onGoBack, onGoForward } = renderHeader(false, { canGoBack: true, canGoForward: true });
    fireEvent.click(screen.getByTitle("Back (⌘←)"));
    fireEvent.click(screen.getByTitle("Forward (⌘→)"));
    expect(onGoBack).toHaveBeenCalledOnce();
    expect(onGoForward).toHaveBeenCalledOnce();
  });
});
