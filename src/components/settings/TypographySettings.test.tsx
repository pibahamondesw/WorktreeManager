// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { TypographySettings } from "./TypographySettings";
import { defaultTypography } from "../../appearance";

afterEach(cleanup);

function renderSettings() {
  const onTypographyChange = vi.fn();
  const onContrastChange = vi.fn();
  render(
    <TypographySettings
      typography={defaultTypography()}
      onTypographyChange={onTypographyChange}
      contrast={100}
      onContrastChange={onContrastChange}
    />
  );
  return { onTypographyChange, onContrastChange };
}

describe("TypographySettings", () => {
  it("saves a family when the field is left or Enter is pressed, trimmed and only if changed", () => {
    const { onTypographyChange } = renderSettings();
    const code = screen.getByRole("textbox", { name: "Code font family" });
    expect(code).toHaveAttribute("placeholder", "Menlo");

    fireEvent.blur(code);
    expect(onTypographyChange).not.toHaveBeenCalled();

    fireEvent.change(code, { target: { value: " Fira Code " } });
    fireEvent.keyDown(code, { key: "a" });
    expect(onTypographyChange).not.toHaveBeenCalled();
    fireEvent.keyDown(code, { key: "Enter" });
    expect(onTypographyChange).toHaveBeenCalledWith({
      ...defaultTypography(),
      code: { family: "Fira Code", size: 12 },
    });
  });

  it("offers sizes within each area's range and saves the chosen one", () => {
    const { onTypographyChange } = renderSettings();
    const terminal = screen.getByRole("combobox", { name: "Terminal font size" });
    expect(terminal).toHaveValue("12");
    expect(terminal.querySelectorAll("option")).toHaveLength(16);

    fireEvent.change(terminal, { target: { value: "18" } });
    expect(onTypographyChange).toHaveBeenCalledWith({
      ...defaultTypography(),
      terminal: { family: "", size: 18 },
    });
  });

  it("saves the contrast level", () => {
    const { onContrastChange } = renderSettings();
    const contrast = screen.getByRole("slider", { name: /Contrast/ });
    fireEvent.change(contrast, { target: { value: "125" } });
    expect(onContrastChange).toHaveBeenCalledWith(125);
  });
});
