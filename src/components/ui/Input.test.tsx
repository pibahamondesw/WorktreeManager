// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { Input } from "./Input";

afterEach(cleanup);

it("associates each label with a unique input", () => {
  render(
    <>
      <Input label="First name" />
      <Input label="Last name" />
    </>
  );
  const first = screen.getByRole("textbox", { name: "First name" });
  const last = screen.getByRole("textbox", { name: "Last name" });
  expect(first.id).not.toBe(last.id);
  expect(first.id).not.toBe("");
});

it("preserves a caller-provided input id", () => {
  render(<Input label="Branch" id="branch-name" />);
  expect(screen.getByRole("textbox", { name: "Branch" })).toHaveAttribute("id", "branch-name");
});
