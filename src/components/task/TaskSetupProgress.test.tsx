// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { TaskSetupProgress } from "./TaskSetupProgress";
import {
  clearTaskSetup,
  initializeTaskSetup,
  getTaskSetup,
  updateSetupStep,
  updateTaskSetup,
} from "../../services/taskSetup";
import { Task } from "../../types";

afterEach(() => {
  cleanup();
  clearTaskSetup("progress-test");
});

it("keeps setup progress across navigation and distinguishes warnings and skipped steps", () => {
  const view = render(<TaskSetupProgress taskId="progress-test" />);
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  act(() =>
    initializeTaskSetup(
      { id: "progress-test", members: [{ repoId: "api", repoName: "API" }] } as Task,
      []
    )
  );
  expect(screen.getByRole("status")).toHaveTextContent("Setup running");
  act(() => {
    updateTaskSetup("progress-test", { ...getTaskSetup("progress-test")!, opened: true });
    updateSetupStep("progress-test", "api", "install_python_deps", "skipped", []);
  });
  expect(screen.getByRole("status")).toHaveTextContent("Workspace opened · Setup running");
  expect(screen.getByText("API · Installing Python dependencies · skipped")).toBeInTheDocument();
  view.unmount();
  render(<TaskSetupProgress taskId="progress-test" />);
  expect(screen.getByRole("status")).toHaveTextContent("Setup running");
  act(() =>
    updateTaskSetup("progress-test", {
      ...getTaskSetup("progress-test")!,
      active: false,
      warnings: [{ repoId: "api", stage: "config", message: "Configuration could not be copied." }],
    })
  );
  expect(screen.getByRole("status")).toHaveTextContent("Setup completed with warnings");
  expect(screen.getByText("API · Configuration could not be copied.")).toBeInTheDocument();
});

it.each([true, false])(
  "dismisses progress while active=%s without losing setup state or resurfacing on navigation",
  (active) => {
    initializeTaskSetup(
      { id: "progress-test", members: [{ repoId: "api", repoName: "API" }] } as Task,
      []
    );
    updateTaskSetup("progress-test", { ...getTaskSetup("progress-test")!, active });
    const view = render(<TaskSetupProgress taskId="progress-test" />);
    fireEvent.click(screen.getByRole("button", { name: "Dismiss setup progress" }));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(getTaskSetup("progress-test")).toMatchObject({ active, dismissed: true });
    act(() => {
      updateSetupStep("progress-test", "api", "install_node_deps", "completed", []);
      updateTaskSetup("progress-test", { ...getTaskSetup("progress-test")!, active: false });
    });
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(getTaskSetup("progress-test")?.repos[0].steps).toContainEqual({
      stage: "install_node_deps",
      status: "completed",
    });
    view.unmount();
    render(<TaskSetupProgress taskId="progress-test" />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  }
);
