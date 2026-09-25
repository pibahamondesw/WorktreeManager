// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { usePresence } from "./usePresence";

afterEach(cleanup);

function Panel({ visible }: { visible: boolean }) {
  const { rendered, exiting, onAnimationEnd } = usePresence(visible);
  if (!rendered) return null;
  return (
    <div data-testid="panel" data-exiting={exiting} onAnimationEnd={onAnimationEnd}>
      <span data-testid="child" />
    </div>
  );
}

function animationEnd(element: HTMLElement) {
  fireEvent(element, new Event("webkitAnimationEnd", { bubbles: true }));
}

it("keeps a hidden panel mounted until its own exit animation ends", () => {
  const view = render(<Panel visible />);
  view.rerender(<Panel visible={false} />);
  expect(screen.getByTestId("panel")).toHaveAttribute("data-exiting", "true");
  animationEnd(screen.getByTestId("child"));
  expect(screen.getByTestId("panel")).toBeInTheDocument();
  animationEnd(screen.getByTestId("panel"));
  expect(screen.queryByTestId("panel")).not.toBeInTheDocument();
});

it("cancels the exit when shown again mid-animation", () => {
  const view = render(<Panel visible />);
  view.rerender(<Panel visible={false} />);
  view.rerender(<Panel visible />);
  animationEnd(screen.getByTestId("panel"));
  expect(screen.getByTestId("panel")).toHaveAttribute("data-exiting", "false");
});
