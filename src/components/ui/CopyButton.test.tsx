// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { CopyButton } from "./CopyButton";

const writeText = vi.fn();

beforeEach(() => {
  vi.useFakeTimers();
  writeText.mockReset();
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

async function clickCopy() {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Copy command" }));
  });
}

it("copies the value and anchors the confirmation to the button", async () => {
  writeText.mockResolvedValue(undefined);
  render(<CopyButton value="brew install git">Copy command</CopyButton>);
  await clickCopy();
  expect(writeText).toHaveBeenCalledWith("brew install git");
  const status = screen.getByRole("status");
  expect(status).toHaveTextContent("Copied");
  expect(status.parentElement).toContainElement(screen.getByRole("button"));
});

it("hides the confirmation after a moment", async () => {
  writeText.mockResolvedValue(undefined);
  render(<CopyButton value="x">Copy command</CopyButton>);
  await clickCopy();
  act(() => {
    vi.advanceTimersByTime(1500);
  });
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
});

it("restarts the confirmation when copied again", async () => {
  writeText.mockResolvedValue(undefined);
  render(<CopyButton value="x">Copy command</CopyButton>);
  await clickCopy();
  act(() => {
    vi.advanceTimersByTime(1000);
  });
  await clickCopy();
  act(() => {
    vi.advanceTimersByTime(1000);
  });
  expect(screen.getByRole("status")).toHaveTextContent("Copied");
});

it("reports a failed copy", async () => {
  writeText.mockRejectedValue(new Error("denied"));
  render(<CopyButton value="x">Copy command</CopyButton>);
  await clickCopy();
  expect(screen.getByRole("status")).toHaveTextContent("Could not copy");
  act(() => {
    vi.advanceTimersByTime(1500);
  });
  expect(screen.getByRole("status")).toHaveTextContent("Could not copy");
});
