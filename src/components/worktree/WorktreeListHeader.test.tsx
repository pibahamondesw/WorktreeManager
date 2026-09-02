// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { WorktreeListHeader } from "./WorktreeListHeader";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(false) }));

function renderHeader(sidebarCollapsed: boolean) {
  const onExpandSidebar = vi.fn();
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
    />
  );
  return onExpandSidebar;
}

afterEach(cleanup);

describe("WorktreeListHeader sidebar toggle", () => {
  it("hides the expand button while the sidebar is visible", () => {
    renderHeader(false);
    expect(screen.queryByTitle("Expand sidebar [")).not.toBeInTheDocument();
  });

  it("expands the sidebar when the button is clicked", () => {
    const onExpandSidebar = renderHeader(true);
    fireEvent.click(screen.getByTitle("Expand sidebar ["));
    expect(onExpandSidebar).toHaveBeenCalledOnce();
  });
});
