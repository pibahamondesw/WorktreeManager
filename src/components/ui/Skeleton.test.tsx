// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { Skeleton } from "./Skeleton";

afterEach(cleanup);

it("renders a pulsing placeholder hidden from assistive technology", () => {
  const { container } = render(<Skeleton className="h-3 w-16" />);
  const placeholder = container.firstElementChild;
  expect(placeholder).toHaveAttribute("aria-hidden", "true");
  expect(placeholder).toHaveClass("motion-pulse-steps", "rounded", "h-3", "w-16");
});

it("rounds pills fully", () => {
  const { container } = render(<Skeleton shape="pill" />);
  expect(container.firstElementChild).toHaveClass("rounded-full");
  expect(container.firstElementChild).not.toHaveClass("rounded");
});
