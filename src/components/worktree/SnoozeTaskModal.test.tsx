// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { SnoozeTaskModal } from "./SnoozeTaskModal";

beforeEach(() => {
  vi.useFakeTimers({ now: new Date(2026, 1, 1, 15, 30), toFake: ["Date"] });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function renderModal(onSnooze = vi.fn().mockResolvedValue(undefined)) {
  const onClose = vi.fn();
  render(<SnoozeTaskModal onSnooze={onSnooze} onClose={onClose} />);
  return { onSnooze, onClose, input: screen.getByLabelText("Snooze until") };
}

it("starts at tomorrow and does not snooze into the past", () => {
  const { onSnooze, input } = renderModal();
  expect(input).toHaveValue("2026-02-02");
  expect(input).toHaveAttribute("min", "2026-02-02");
  fireEvent.change(input, { target: { value: "2026-02-01" } });
  expect(screen.getByRole("button", { name: "Snooze" })).toBeDisabled();
  fireEvent.submit(input.closest("form")!);
  expect(onSnooze).not.toHaveBeenCalled();
});

it("snoozes until the start of the picked day and closes", async () => {
  const { onSnooze, onClose, input } = renderModal();
  fireEvent.change(input, { target: { value: "2026-03-10" } });
  fireEvent.click(screen.getByRole("button", { name: "Snooze" }));
  expect(onSnooze).toHaveBeenCalledWith(new Date(2026, 2, 10).toISOString());
  await waitFor(() => expect(onClose).toHaveBeenCalled());
});

it("keeps the dialog open with the error when snoozing fails", async () => {
  const { onClose } = renderModal(vi.fn().mockRejectedValue(new Error("disk full")));
  fireEvent.click(screen.getByRole("button", { name: "Snooze" }));
  expect(await screen.findByText("Error: disk full")).toBeInTheDocument();
  expect(onClose).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Snooze" })).toBeEnabled();
});

it("closes on cancel", () => {
  const { onClose } = renderModal();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onClose).toHaveBeenCalled();
});

it("does not snooze until a day that already started while the dialog was open", () => {
  const { onSnooze, input } = renderModal();
  vi.setSystemTime(new Date(2026, 1, 2, 0, 1));
  fireEvent.submit(input.closest("form")!);
  expect(onSnooze).not.toHaveBeenCalled();
});
