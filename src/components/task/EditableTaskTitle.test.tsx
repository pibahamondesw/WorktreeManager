// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { EditableTaskTitle } from "./EditableTaskTitle";

afterEach(cleanup);

function renderTitle(onRename?: (title: string) => Promise<unknown>, onError = vi.fn()) {
  const onParentClick = vi.fn();
  render(
    <div onClick={onParentClick}>
      <EditableTaskTitle title="Old title" textClassName="" onRename={onRename} onError={onError} />
    </div>
  );
  return { onParentClick, onError };
}

function startEditing() {
  fireEvent.click(screen.getByRole("button", { name: "Rename task" }));
  return screen.getByRole("textbox", { name: "Task title" });
}

it("hides the pencil when the task cannot be renamed", () => {
  renderTitle();
  expect(screen.queryByRole("button", { name: "Rename task" })).not.toBeInTheDocument();
});

it("saves the trimmed title on Enter without opening the card", async () => {
  const onRename = vi.fn().mockResolvedValue(undefined);
  const { onParentClick } = renderTitle(onRename);
  const input = startEditing();
  fireEvent.change(input, { target: { value: "  New title " } });
  await act(async () => {
    fireEvent.keyDown(input, { key: "Enter" });
  });
  expect(onRename).toHaveBeenCalledWith("New title");
  expect(onParentClick).not.toHaveBeenCalled();
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
});

it("cancels on Escape without saving or bubbling the key to app shortcuts", () => {
  const onRename = vi.fn();
  const onWindowKey = vi.fn();
  window.addEventListener("keydown", onWindowKey);
  renderTitle(onRename);
  const input = startEditing();
  fireEvent.change(input, { target: { value: "New title" } });
  fireEvent.keyDown(input, { key: "Escape" });
  window.removeEventListener("keydown", onWindowKey);
  expect(onRename).not.toHaveBeenCalled();
  expect(onWindowKey).not.toHaveBeenCalled();
  expect(screen.getByText("Old title")).toBeInTheDocument();
});

it("reverts to the previous title and reports the error when saving fails", async () => {
  const onRename = vi.fn().mockRejectedValue(new Error("Could not rename the Linear issue."));
  const { onError } = renderTitle(onRename);
  const input = startEditing();
  fireEvent.change(input, { target: { value: "New title" } });
  await act(async () => {
    fireEvent.keyDown(input, { key: "Enter" });
  });
  expect(onError).toHaveBeenCalledWith("Could not rename the Linear issue.");
  expect(screen.getByText("Old title")).toBeInTheDocument();
});
