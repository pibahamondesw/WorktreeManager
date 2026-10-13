// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { HistoryMenu } from "./HistoryMenu";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

it.each([
  ["Fork conversation from here", "conversation"],
  ["Rewind code to here", "code"],
  ["Fork conversation and rewind code", "both"],
])("selects %s from the icon menu", (label, action) => {
  const onPick = vi.fn();
  render(<HistoryMenu canRewind onPick={onPick} />);
  const button = screen.getByRole("button", { name: "Rewind or fork" });
  expect(button).toHaveTextContent("");
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  fireEvent.click(button);
  expect(button).toHaveAttribute("aria-expanded", "true");
  fireEvent.click(screen.getByRole("menuitem", { name: label }));
  expect(onPick).toHaveBeenCalledExactlyOnceWith(action);
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  expect(button).toHaveFocus();
});

it("supports keyboard navigation and dismisses Escape without triggering app shortcuts", () => {
  render(<HistoryMenu canRewind onPick={vi.fn()} />);
  const button = screen.getByRole("button", { name: "Rewind or fork" });
  fireEvent.keyDown(button, { key: "ArrowDown" });
  const options = screen.getAllByRole("menuitem");
  expect(options[0]).toHaveFocus();
  fireEvent.keyDown(options[0], { key: "ArrowDown" });
  expect(options[1]).toHaveFocus();
  fireEvent.keyDown(options[1], { key: "End" });
  expect(options[2]).toHaveFocus();
  fireEvent.keyDown(options[2], { key: "ArrowDown" });
  expect(options[0]).toHaveFocus();
  const shortcut = vi.fn();
  window.addEventListener("keydown", shortcut);
  fireEvent.keyDown(options[0], { key: "Escape" });
  window.removeEventListener("keydown", shortcut);
  expect(shortcut).not.toHaveBeenCalled();
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  expect(button).toHaveFocus();
});

it("keeps code actions unavailable without a checkpoint and skips them with the keyboard", () => {
  render(<HistoryMenu canRewind={false} onPick={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Rewind or fork" }));
  const fork = screen.getByRole("menuitem", { name: "Fork conversation from here" });
  expect(screen.getByRole("menuitem", { name: "Rewind code to here" })).toBeDisabled();
  expect(
    screen.getByRole("menuitem", { name: "Fork conversation and rewind code" })
  ).toBeDisabled();
  fireEvent.keyDown(fork, { key: "ArrowDown" });
  expect(fork).toHaveFocus();
});

it("closes on outside clicks and scrolling", () => {
  render(<HistoryMenu canRewind onPick={vi.fn()} />);
  const button = screen.getByRole("button", { name: "Rewind or fork" });
  fireEvent.click(button);
  fireEvent.pointerDown(document.body);
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  fireEvent.click(button);
  fireEvent.scroll(window);
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
});

it("closes when the session becomes busy and stays closed when idle again", () => {
  const onPick = vi.fn();
  const view = render(<HistoryMenu canRewind onPick={onPick} />);
  fireEvent.click(screen.getByRole("button", { name: "Rewind or fork" }));
  view.rerender(<HistoryMenu canRewind disabled onPick={onPick} />);
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  view.rerender(<HistoryMenu canRewind onPick={onPick} />);
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
});

it("opens above messages near the bottom of the viewport", () => {
  const top = window.innerHeight - 12;
  render(<HistoryMenu canRewind onPick={vi.fn()} />);
  const button = screen.getByRole("button", { name: "Rewind or fork" });
  vi.spyOn(button, "getBoundingClientRect").mockReturnValue({
    top,
    bottom: top + 28,
    right: window.innerWidth - 8,
    left: window.innerWidth - 36,
    width: 28,
    height: 28,
    x: 0,
    y: top,
    toJSON: () => ({}),
  });
  fireEvent.click(button);
  expect(screen.getByRole("menu")).toHaveStyle({ top: `${top - 166}px` });
});
