// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { useMouseReorder } from "./useMouseReorder";

afterEach(cleanup);

function List({
  keys,
  onReorder,
  onClick,
}: {
  keys: string[];
  onReorder: (s: string, t: string) => void;
  onClick: (key: string) => void;
}) {
  const reorder = useMouseReorder(onReorder);
  return (
    <ul>
      {keys.map((key) => (
        <li key={key} {...reorder.itemProps(key)} onClick={() => onClick(key)}>
          {key}
          {reorder.over === key && " (over)"}
        </li>
      ))}
    </ul>
  );
}

function setup() {
  const onReorder = vi.fn();
  const onClick = vi.fn();
  render(
    <>
      <List keys={["a", "b"]} onReorder={onReorder} onClick={onClick} />
      <List keys={["x"]} onReorder={onReorder} onClick={onClick} />
    </>
  );
  return { onReorder, onClick };
}

it("reorders onto the item under the mouse and swallows the click that ends the drag", () => {
  const { onReorder, onClick } = setup();
  fireEvent.mouseDown(screen.getByText("a"), { button: 0, clientX: 0, clientY: 0 });
  fireEvent.mouseMove(screen.getByText("b"), { clientX: 0, clientY: 30 });
  expect(screen.getByText("b (over)")).toBeInTheDocument();
  expect(document.body.style.userSelect).toBe("none");
  fireEvent.mouseUp(screen.getByText("b (over)"));
  fireEvent.click(screen.getByText("b"));

  expect(onReorder).toHaveBeenCalledWith("a", "b");
  expect(onClick).not.toHaveBeenCalled();
  expect(document.body.style.userSelect).toBe("");
});

it("treats a press without movement as a click", () => {
  const { onReorder, onClick } = setup();
  fireEvent.mouseDown(screen.getByText("a"), { button: 0 });
  fireEvent.mouseUp(screen.getByText("a"));
  fireEvent.click(screen.getByText("a"));

  expect(onReorder).not.toHaveBeenCalled();
  expect(onClick).toHaveBeenCalledWith("a");
});

it("ignores other buttons, the source itself and items from another list", () => {
  const { onReorder } = setup();
  fireEvent.mouseDown(screen.getByText("a"), { button: 2, clientX: 0, clientY: 0 });
  fireEvent.mouseMove(screen.getByText("b"), { clientX: 0, clientY: 30 });
  fireEvent.mouseUp(screen.getByText("b"));

  for (const target of ["a", "x"]) {
    const item = () => screen.getByText(new RegExp(`^${target}`));
    fireEvent.mouseDown(screen.getByText(/^a/), { button: 0, clientX: 0, clientY: 0 });
    fireEvent.mouseMove(item(), { clientX: 0, clientY: 30 });
    fireEvent.mouseUp(item());
  }

  expect(onReorder).not.toHaveBeenCalled();
});
