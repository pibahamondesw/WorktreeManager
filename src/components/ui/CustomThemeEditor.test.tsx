// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { CustomThemeEditor } from "./CustomThemeEditor";
import { generateThemeColors } from "../../themePalette";
import { themes } from "../../themes";

afterEach(cleanup);

it("generates every token from the chosen background and accent", () => {
  const onChange = vi.fn();
  render(<CustomThemeEditor colors={themes[0].colors} onChange={onChange} />);

  fireEvent.input(screen.getByLabelText("Background color"), { target: { value: "#fdf6e3" } });
  fireEvent.input(screen.getByLabelText("Accent color"), { target: { value: "#268bd2" } });
  fireEvent.click(screen.getByRole("button", { name: "Generate theme" }));

  expect(onChange).toHaveBeenCalledWith(generateThemeColors("#fdf6e3", "#268bd2"));
});

it("starts the generator from the current colors", () => {
  const onChange = vi.fn();
  render(<CustomThemeEditor colors={{}} onChange={onChange} />);

  fireEvent.click(screen.getByRole("button", { name: "Generate theme" }));

  expect(onChange).toHaveBeenCalledWith(generateThemeColors("#0a0a0f", "#6366f1"));
});
