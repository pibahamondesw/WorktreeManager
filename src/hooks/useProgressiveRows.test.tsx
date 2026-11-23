// @vitest-environment jsdom
import { expect, it } from "vitest";
import { renderHook } from "@testing-library/react";
import { useProgressiveRows } from "./useProgressiveRows";

const rowsOf = (count: number) => Array.from({ length: count }, (_, index) => index);

function renderedLengths(initial: number[]) {
  const lengths: number[] = [];
  const view = renderHook(
    ({ rows }) => {
      const rendered = useProgressiveRows(rows, 3);
      lengths.push(rendered.length);
      return rendered;
    },
    { initialProps: { rows: initial } }
  );
  return { lengths, view };
}

it("paints the first rows on mount and the rest in a deferred render", () => {
  const { lengths, view } = renderedLengths(rowsOf(10));
  expect(lengths[0]).toBe(3);
  expect(view.result.current).toEqual(rowsOf(10));
});

it("paints short lists whole on mount", () => {
  const { lengths } = renderedLengths(rowsOf(2));
  expect(lengths).toEqual([2]);
});

it("paints every row at once when the list grows or shrinks after mount", () => {
  const { lengths, view } = renderedLengths(rowsOf(10));
  lengths.length = 0;
  view.rerender({ rows: rowsOf(20) });
  view.rerender({ rows: rowsOf(5) });
  expect(lengths).toEqual([20, 5]);
});
