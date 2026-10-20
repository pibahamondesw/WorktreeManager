// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { ThemeSettings } from "./ThemePicker";

afterEach(cleanup);

it("groups presets by appearance around the system and custom options", () => {
  const onThemeChange = vi.fn();
  render(
    <ThemeSettings
      currentThemeId="system"
      onThemeChange={onThemeChange}
      customColors={null}
      onCustomColorsChange={vi.fn()}
    />
  );

  const section = (title: string) =>
    screen.getByRole("heading", { name: title }).closest("section")!;
  expect(within(section("Light")).getByRole("button", { name: /Solarized Light/ })).toBeVisible();
  expect(within(section("Dark")).getByRole("button", { name: /Dracula/ })).toBeVisible();
  expect(within(section("Light")).queryByRole("button", { name: /Dracula/ })).toBeNull();

  fireEvent.click(within(section("Automatic")).getByRole("button", { name: /System/ }));
  expect(onThemeChange).toHaveBeenCalledWith("system");
  fireEvent.click(within(section("Light")).getByRole("button", { name: /Gruvbox Light/ }));
  expect(onThemeChange).toHaveBeenCalledWith("gruvbox-light");
});
