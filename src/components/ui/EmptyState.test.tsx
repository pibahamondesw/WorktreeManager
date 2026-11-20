// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { EmptyState } from "./EmptyState";

afterEach(cleanup);

it("shows the title alone when there is no action", () => {
  const { container } = render(<EmptyState title="No runs yet" />);
  expect(screen.getByText("No runs yet")).toBeInTheDocument();
  expect(container.firstElementChild?.children).toHaveLength(1);
});

it("offers the action below the title", () => {
  render(<EmptyState title="No workspaces yet" action={<button>Add one</button>} />);
  expect(screen.getByRole("button", { name: "Add one" })).toBeInTheDocument();
});
