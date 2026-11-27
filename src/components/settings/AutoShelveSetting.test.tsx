// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { AutoShelveSetting } from "./AutoShelveSetting";

afterEach(cleanup);

function renderSetting(inactiveDays: number | null) {
  const onChange = vi.fn();
  render(<AutoShelveSetting inactiveDays={inactiveDays} onChange={onChange} />);
  return {
    onChange,
    toggle: screen.getByRole("checkbox", { name: /Move tasks inactive for/ }),
    days: screen.getByLabelText("Days without activity"),
  };
}

it("saves a new number of days when the field loses focus", () => {
  const { onChange, days } = renderSetting(30);
  fireEvent.change(days, { target: { value: "14" } });
  fireEvent.blur(days);
  expect(onChange).toHaveBeenCalledWith(14);
});

it("restores the saved days instead of saving an invalid count", () => {
  const { onChange, days } = renderSetting(30);
  fireEvent.change(days, { target: { value: "0" } });
  fireEvent.blur(days);
  expect(onChange).not.toHaveBeenCalled();
  expect(days).toHaveValue(30);
});

it("does not save unchanged days", () => {
  const { onChange, days } = renderSetting(30);
  fireEvent.blur(days);
  expect(onChange).not.toHaveBeenCalled();
});

it("turns the inactivity rule off and back on with the shown days", () => {
  const { onChange, toggle } = renderSetting(30);
  expect(toggle).toBeChecked();
  fireEvent.click(toggle);
  expect(onChange).toHaveBeenLastCalledWith(null);
  cleanup();
  const off = renderSetting(null);
  expect(off.days).toBeDisabled();
  fireEvent.click(off.toggle);
  expect(off.onChange).toHaveBeenLastCalledWith(30);
});

it("turns the rule on with the default when the shown days are invalid", () => {
  const { onChange, toggle, days } = renderSetting(null);
  fireEvent.change(days, { target: { value: "" } });
  fireEvent.click(toggle);
  expect(onChange).toHaveBeenLastCalledWith(30);
});
