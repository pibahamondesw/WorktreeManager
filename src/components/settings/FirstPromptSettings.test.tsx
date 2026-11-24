// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { FirstPromptSettings } from "./FirstPromptSettings";

afterEach(cleanup);

const template = () => screen.getByRole("textbox", { name: "Template" });
const save = () => screen.getByRole("button", { name: "Save template" });

it("saves an edited template", async () => {
  const onSave = vi.fn().mockResolvedValue(undefined);
  render(<FirstPromptSettings template="" onSave={onSave} />);
  expect(save()).toBeDisabled();
  fireEvent.change(template(), { target: { value: "Work on {issue.identifier}" } });
  fireEvent.click(save());
  await waitFor(() => expect(onSave).toHaveBeenCalledWith("Work on {issue.identifier}"));
});

it("explains an unknown variable and does not save it", () => {
  const onSave = vi.fn();
  render(<FirstPromptSettings template="" onSave={onSave} />);
  fireEvent.change(template(), { target: { value: "{nope}" } });
  expect(screen.getByText("The template uses the unknown variable {nope}.")).toBeInTheDocument();
  expect(template()).toHaveAttribute("aria-invalid", "true");
  expect(save()).toBeDisabled();
});

it("shows why saving failed", async () => {
  render(
    <FirstPromptSettings template="" onSave={vi.fn().mockRejectedValue(new Error("Disk full"))} />
  );
  fireEvent.change(template(), { target: { value: "Hi" } });
  fireEvent.click(save());
  expect(await screen.findByText("Disk full")).toBeInTheDocument();
});
