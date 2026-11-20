// @vitest-environment jsdom
import { expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { Kbd } from "./Kbd";

it("renders the keys as a keyboard element", () => {
  render(<Kbd>⌘K</Kbd>);
  expect(screen.getByText("⌘K").tagName).toBe("KBD");
});
