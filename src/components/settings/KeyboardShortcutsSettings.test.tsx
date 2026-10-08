// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { KeyboardShortcutsSettings } from "./KeyboardShortcutsSettings";
import {
  isShortcutCaptureActive,
  setShortcutOverrides,
  useShortcutActions,
} from "../../shortcuts/runtime";
import { shortcut } from "../../shortcuts/catalog";
afterEach(() => {
  cleanup();
  setShortcutOverrides({});
});
function mount() {
  const operations = {
    updateShortcut: vi.fn().mockResolvedValue(undefined),
    resetShortcuts: vi.fn().mockResolvedValue(undefined),
  };
  render(<KeyboardShortcutsSettings operations={operations} workspaceCount={2} taskCount={1} />);
  return operations;
}
async function record(key: string, modifiers: KeyboardEventInit = {}) {
  fireEvent.click(screen.getByRole("button", { name: "Change New task (Task list)" }));
  fireEvent.click(screen.getByRole("button", { name: "Add alternative" }));
  await waitFor(() => expect(screen.getByRole("group", { name: "Record shortcut" })).toHaveFocus());
  fireEvent.keyDown(screen.getByRole("group", { name: "Record shortcut" }), { key, ...modifiers });
  fireEvent.keyUp(screen.getByRole("group", { name: "Record shortcut" }), { key, ...modifiers });
  fireEvent.click(screen.getByRole("button", { name: "Save shortcut" }));
}
it("captures without executing commands and confirms reassignment before saving", async () => {
  const action = vi.fn();
  function Actions() {
    useShortcutActions({ "list.linear": { handler: action } });
    return null;
  }
  render(<Actions />);
  const operations = mount();
  await record("l");
  expect(action).not.toHaveBeenCalled();
  expect(operations.updateShortcut).not.toHaveBeenCalled();
  expect(screen.getByRole("alertdialog")).toHaveTextContent("will have no shortcut");
  fireEvent.click(screen.getByRole("button", { name: "Reassign" }));
  await waitFor(() =>
    expect(operations.updateShortcut).toHaveBeenCalledWith(
      "list.new",
      [shortcut("n"), shortcut("l")],
      true
    )
  );
  await waitFor(() => expect(isShortcutCaptureActive()).toBe(false));
});
it("does not write duplicates and Escape cancels capture", async () => {
  const operations = mount();
  await record("n");
  expect(screen.getByRole("alert")).toHaveTextContent("Already assigned");
  expect(operations.updateShortcut).not.toHaveBeenCalled();
  fireEvent.keyDown(screen.getByRole("group", { name: "Record shortcut" }), { key: "Escape" });
  await waitFor(() =>
    expect(screen.queryByRole("group", { name: "Record shortcut" })).not.toBeInTheDocument()
  );
  expect(isShortcutCaptureActive()).toBe(false);
});
it("blocks protected navigation and simultaneous letters", async () => {
  mount();
  await record("Enter");
  expect(screen.getByRole("alert")).toHaveTextContent("Reserved");
  const recorder = screen.getByRole("group", { name: "Record shortcut" });
  fireEvent.keyDown(recorder, { key: "k", code: "KeyK", metaKey: true });
  fireEvent.keyDown(recorder, { key: "r", code: "KeyR", metaKey: true });
  expect(screen.getByRole("alert")).toHaveTextContent("single key");
  expect(screen.getByRole("button", { name: "Save shortcut" })).toBeDisabled();
});
it("shows a failed save without closing the recorder or changing bindings", async () => {
  const operations = mount();
  operations.updateShortcut.mockRejectedValue(new Error("Disk unavailable"));
  await record("y", { metaKey: true });
  await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Disk unavailable"));
  expect(screen.getByRole("group", { name: "Record shortcut" })).toBeInTheDocument();
});
it("filters customized and unassigned entries and confirms restore all", async () => {
  act(() => setShortcutOverrides({ "list.new": [] }));
  const operations = mount();
  fireEvent.change(screen.getByRole("combobox", { name: "Filter shortcuts" }), {
    target: { value: "unassigned" },
  });
  expect(screen.getByText("Unassigned", { selector: "span" })).toBeInTheDocument();
  expect(screen.queryByText("Copy branch name")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Restore all" }));
  expect(operations.resetShortcuts).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Confirm restore" }));
  await waitFor(() => expect(operations.resetShortcuts).toHaveBeenCalledOnce());
});

it("finds bindings by conventional combination text", () => {
  mount();
  fireEvent.change(screen.getByRole("textbox", { name: "Search shortcuts" }), {
    target: { value: "cmd+k" },
  });
  expect(screen.getByText("Search tasks")).toBeInTheDocument();
  expect(screen.queryByText("Copy branch name")).not.toBeInTheDocument();
});

it("shows compact read-only key badges with a single editing entry point", () => {
  mount();
  const row = screen.getAllByRole("group", { name: "Open settings" })[0];
  expect(within(row).getByText("Default")).toBeInTheDocument();
  expect(within(row).getByText("⌘S").tagName).toBe("KBD");
  expect(within(row).getAllByRole("button")).toHaveLength(1);
  expect(within(row).getByRole("button", { name: "Change Open settings (General)" })).toHaveClass(
    "bg-accent"
  );
  expect(row).not.toHaveTextContent("Task list · Task view · Chat");
  expect(row).not.toHaveTextContent("Default:");
  expect(screen.queryByRole("button", { name: "Add alternative" })).not.toBeInTheDocument();
});

it("exposes replace and remove only after Change and preserves replacement semantics", async () => {
  const operations = mount();
  fireEvent.click(screen.getByRole("button", { name: "Change New task (Task list)" }));
  expect(screen.getByRole("button", { name: "Remove N" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Replace N" }));
  const recorder = screen.getByRole("group", { name: "Record shortcut" });
  await waitFor(() => expect(recorder).toHaveFocus());
  fireEvent.keyDown(recorder, { key: "y", metaKey: true });
  fireEvent.keyUp(recorder, { key: "y", metaKey: true });
  fireEvent.click(screen.getByRole("button", { name: "Save shortcut" }));
  await waitFor(() =>
    expect(operations.updateShortcut).toHaveBeenCalledWith("list.new", [shortcut("y", "meta")])
  );
});

it("shows original bindings only in the restore tooltip for customized rows", async () => {
  act(() => setShortcutOverrides({ "list.new": [shortcut("y")] }));
  const operations = mount();
  const row = screen.getAllByRole("group", { name: "New task" })[0];
  expect(within(row).getByText("Custom")).toHaveClass("text-accent");
  const restore = within(row).getByRole("button", {
    name: "Restore defaults for New task (Task list)",
  });
  expect(restore).toHaveAttribute("title", "Restore defaults: N");
  expect(within(row).queryByText("N")).not.toBeInTheDocument();
  fireEvent.click(restore);
  await waitFor(() => expect(operations.updateShortcut).toHaveBeenCalledWith("list.new", null));
});

it("collapses sections independently and expands matching sections during search", () => {
  mount();
  const list = screen.getByRole("button", { name: /^Task list/ });
  const chat = screen.getByRole("button", { name: /^Chat/ });
  expect(list).toHaveAttribute("aria-expanded", "true");
  fireEvent.click(list);
  expect(list).toHaveAttribute("aria-expanded", "false");
  expect(chat).toHaveAttribute("aria-expanded", "true");
  expect(
    screen.queryByRole("button", { name: "Change New task (Task list)" })
  ).not.toBeInTheDocument();
  const search = screen.getByRole("textbox", { name: "Search shortcuts" });
  fireEvent.change(search, { target: { value: "new task" } });
  expect(screen.getByRole("button", { name: /^Task list/ })).toHaveAttribute(
    "aria-expanded",
    "true"
  );
  expect(screen.getByRole("button", { name: "Change New task (Task list)" })).toBeVisible();
  fireEvent.change(search, { target: { value: "" } });
  expect(screen.getByRole("button", { name: /^Task list/ })).toHaveAttribute(
    "aria-expanded",
    "false"
  );
});

it("releases recording when its section is collapsed", async () => {
  mount();
  fireEvent.click(screen.getByRole("button", { name: "Change New task (Task list)" }));
  fireEvent.click(screen.getByRole("button", { name: "Add alternative" }));
  await waitFor(() => expect(isShortcutCaptureActive()).toBe(true));
  fireEvent.click(screen.getByRole("button", { name: /^Task list/ }));
  await waitFor(() => expect(isShortcutCaptureActive()).toBe(false));
  expect(screen.queryByRole("group", { name: "Record shortcut" })).not.toBeInTheDocument();
});

it("shows the primary default while retaining alternatives in the editor", () => {
  mount();
  const row = screen.getAllByRole("group", { name: "Open settings" })[0];
  expect(within(row).getByText("⌘S")).toBeVisible();
  expect(within(row).queryByText("⌃S")).not.toBeInTheDocument();
  fireEvent.click(within(row).getByRole("button", { name: "Change Open settings (General)" }));
  expect(screen.getByRole("button", { name: "Replace ⌃S" })).toBeVisible();
});

it("compacts all positional shortcuts into non-editable ranges", () => {
  mount();
  expect(screen.getAllByRole("group", { name: "Switch to workspace" })).toHaveLength(2);
  for (const name of ["Switch to workspace", "Jump to task", "Jump to task result"]) {
    for (const row of screen.getAllByRole("group", { name })) {
      expect(within(row).queryByRole("button")).not.toBeInTheDocument();
      expect(row).toHaveTextContent("0–9");
    }
  }
  expect(screen.queryByText("Switch to workspace 0")).not.toBeInTheDocument();
  const numbered = screen.getByRole("group", { name: "Select numbered issue or option" });
  expect(within(numbered).getByText("0–9").tagName).toBe("KBD");
  expect(within(numbered).queryByText("0")).not.toBeInTheDocument();
});
