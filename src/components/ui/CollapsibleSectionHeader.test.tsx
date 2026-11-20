// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { CollapsibleSectionHeader } from "./CollapsibleSectionHeader";

afterEach(cleanup);

it("announces the expanded section it controls", () => {
  render(
    <CollapsibleSectionHeader expanded onToggle={() => undefined} controls="section-body">
      Navigation
    </CollapsibleSectionHeader>
  );
  const header = screen.getByRole("button", { name: /Navigation/ });
  expect(header).toHaveAttribute("aria-expanded", "true");
  expect(header).toHaveAttribute("aria-controls", "section-body");
});

it("toggles on click", () => {
  const onToggle = vi.fn();
  render(
    <CollapsibleSectionHeader expanded={false} onToggle={onToggle} controls="section-body">
      Navigation
    </CollapsibleSectionHeader>
  );
  const header = screen.getByRole("button", { name: /Navigation/ });
  expect(header).toHaveAttribute("aria-expanded", "false");
  fireEvent.click(header);
  expect(onToggle).toHaveBeenCalledOnce();
});
