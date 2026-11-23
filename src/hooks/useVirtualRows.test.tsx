// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

vi.mock("../observeResize", () => import("../test/fakeObserveResize"));

import { rowsBetween, useVirtualRows } from "./useVirtualRows";
import { resize } from "../test/fakeObserveResize";
import { layOutScroller } from "../test/fakeLayout";

const ROW_HEIGHT = 120;
const VIEWPORT = 600;
const keysUpTo = (count: number) => Array.from({ length: count }, (_, index) => `r${index}`);

function Timeline({ keys }: { keys: string[] }) {
  const { scrollRef, sectionRef, rowRef, start, end, before, after, ...handlers } =
    useVirtualRows(keys);
  return (
    <>
      <div ref={scrollRef} onScroll={handlers.onScroll} data-testid="scroller">
        <div>
          <div ref={sectionRef} onFocus={handlers.onFocus} onBlur={handlers.onBlur}>
            {before > 0 && <div data-testid="before" style={{ height: before }} />}
            {keys.slice(start, end).map((key) => (
              <div key={key} ref={rowRef} data-row-key={key}>
                <button>{key}</button>
              </div>
            ))}
            {after > 0 && <div data-testid="after" style={{ height: after }} />}
          </div>
        </div>
      </div>
      <button onClick={handlers.stickToBottom}>Stick</button>
      <p>Outside</p>
    </>
  );
}

const renderedKeys = () =>
  [...document.querySelectorAll<HTMLElement>("[data-row-key]")].map((row) => row.dataset.rowKey);

function renderLaidOut(keys: string[], rowHeight: (key: string) => number = () => ROW_HEIGHT) {
  const view = render(<Timeline keys={keys} />);
  const scroller = screen.getByTestId("scroller");
  layOutScroller(scroller, VIEWPORT, rowHeight);
  resize();
  return { ...view, scroller };
}

const scrollTo = (scroller: HTMLElement, top: number) => {
  scroller.scrollTop = top;
  fireEvent.scroll(scroller);
};

const selectNodeContents = (node: Node) =>
  act(() => {
    const range = document.createRange();
    range.selectNodeContents(node);
    document.getSelection()!.removeAllRanges();
    document.getSelection()!.addRange(range);
    document.dispatchEvent(new Event("selectionchange"));
  });

const clearSelection = () =>
  act(() => {
    document.getSelection()!.removeAllRanges();
    document.dispatchEvent(new Event("selectionchange"));
  });

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  document.getSelection()?.removeAllRanges();
});

describe("rowsBetween", () => {
  it("returns the rows overlapping the range", () => {
    expect(rowsBetween([0, 100, 200, 300, 400], 150, 250)).toEqual({ start: 1, end: 3 });
    expect(rowsBetween([0, 100, 200], 200, 500)).toEqual({ start: 2, end: 2 });
    expect(rowsBetween([0], 0, 100)).toEqual({ start: 0, end: 0 });
  });
});

describe("useVirtualRows", () => {
  it("renders the latest rows before the scroller is laid out", () => {
    render(<Timeline keys={keysUpTo(100)} />);
    expect(renderedKeys()).toEqual(keysUpTo(100).slice(70));
  });

  it("renders the rows near the bottom and stays there as rows arrive", () => {
    const { scroller, rerender } = renderLaidOut(keysUpTo(100));
    expect(renderedKeys()).toEqual(keysUpTo(100).slice(90));
    expect(screen.getByTestId("before")).toHaveStyle({ height: `${90 * ROW_HEIGHT}px` });
    expect(scroller.scrollTop).toBe(100 * ROW_HEIGHT - VIEWPORT);

    rerender(<Timeline keys={keysUpTo(101)} />);
    expect(screen.getByRole("button", { name: "r100" })).toBeInTheDocument();
    expect(scroller.scrollTop).toBe(101 * ROW_HEIGHT - VIEWPORT);
  });

  it("renders the rows around the viewport after scrolling up", () => {
    const { scroller, rerender } = renderLaidOut(keysUpTo(100));
    scrollTo(scroller, 0);
    expect(renderedKeys()).toEqual(keysUpTo(10));
    expect(screen.getByTestId("after")).toHaveStyle({ height: `${90 * ROW_HEIGHT}px` });

    rerender(<Timeline keys={keysUpTo(101)} />);
    expect(scroller.scrollTop).toBe(0);

    scrollTo(scroller, 6000);
    expect(renderedKeys()).toEqual(keysUpTo(100).slice(45, 60));

    fireEvent.click(screen.getByRole("button", { name: "Stick" }));
    expect(renderedKeys()).toEqual(keysUpTo(101).slice(91));
    expect(scroller.scrollTop).toBe(101 * ROW_HEIGHT - VIEWPORT);
  });

  it("keeps the first visible row in place when a row above it grows", () => {
    const heights: Record<string, number> = {};
    const { scroller } = renderLaidOut(keysUpTo(100), (key) => heights[key] ?? ROW_HEIGHT);
    scrollTo(scroller, 6050);
    heights.r46 = 320;
    resize([document.querySelector('[data-row-key="r46"]')!]);
    expect(scroller.scrollTop).toBe(6250);

    heights.r52 = 320;
    resize([document.querySelector('[data-row-key="r52"]')!]);
    expect(scroller.scrollTop).toBe(6250);
  });

  it("keeps the first visible row in place when older rows are prepended", () => {
    const keys = keysUpTo(100);
    const { scroller, rerender } = renderLaidOut(keys);
    scrollTo(scroller, 6050);
    const older = Array.from({ length: 50 }, (_, index) => `older${index}`);
    rerender(<Timeline keys={[...older, ...keys]} />);
    expect(scroller.scrollTop).toBe(6050 + 50 * ROW_HEIGHT);
    expect(document.querySelector('[data-row-key="r50"]')!.getBoundingClientRect().top).toBe(-50);
  });

  it("does not undo a scroll the scroller has not reported yet", () => {
    const heights: Record<string, number> = {};
    const { scroller, rerender } = renderLaidOut(
      keysUpTo(100),
      (key) => heights[key] ?? ROW_HEIGHT
    );
    scrollTo(scroller, 6050);
    scroller.scrollTop = 6100;
    rerender(<Timeline keys={keysUpTo(100)} />);
    expect(scroller.scrollTop).toBe(6100);

    heights.r46 = 320;
    resize([document.querySelector('[data-row-key="r46"]')!]);
    expect(scroller.scrollTop).toBe(6300);
  });

  it("stays at the bottom when a row grows before its resize is reported", () => {
    const heights: Record<string, number> = {};
    const { scroller } = renderLaidOut(keysUpTo(100), (key) => heights[key] ?? ROW_HEIGHT);
    heights.r99 = 420;
    fireEvent.scroll(scroller);
    resize([document.querySelector('[data-row-key="r99"]')!]);
    expect(scroller.scrollTop).toBe(100 * ROW_HEIGHT + 300 - VIEWPORT);
  });

  it("keeps its rows while the scroller is hidden", () => {
    let hidden = false;
    const { scroller } = renderLaidOut(keysUpTo(100), () => (hidden ? 0 : ROW_HEIGHT));
    scrollTo(scroller, 0);
    hidden = true;
    Object.defineProperty(scroller, "clientHeight", { configurable: true, value: 0 });
    resize();
    expect(renderedKeys()).toEqual(keysUpTo(10));
    expect(screen.getByTestId("after")).toHaveStyle({ height: `${90 * ROW_HEIGHT}px` });
  });

  it("keeps the rows of a text selection mounted", () => {
    const { scroller } = renderLaidOut(keysUpTo(100));
    scrollTo(scroller, 0);
    selectNodeContents(screen.getByRole("button", { name: "r2" }));
    scrollTo(scroller, 100 * ROW_HEIGHT - VIEWPORT);
    expect(renderedKeys()).toEqual(keysUpTo(100).slice(2));

    clearSelection();
    expect(renderedKeys()).toEqual(keysUpTo(100).slice(90));
  });

  it("mounts every row a selection from outside the timeline spans", () => {
    const { scroller } = renderLaidOut(keysUpTo(100));
    scrollTo(scroller, 6000);
    selectNodeContents(document.body);
    expect(renderedKeys()).toEqual(keysUpTo(100));

    selectNodeContents(screen.getByText("Outside"));
    expect(renderedKeys()).toEqual(keysUpTo(100).slice(45, 60));
  });

  it("keeps the focused row mounted until the focus leaves the timeline", () => {
    const { scroller } = renderLaidOut(keysUpTo(100));
    scrollTo(scroller, 0);
    act(() => screen.getByRole("button", { name: "r1" }).focus());
    act(() => screen.getByRole("button", { name: "r3" }).focus());
    scrollTo(scroller, 100 * ROW_HEIGHT - VIEWPORT);
    expect(renderedKeys()).toEqual(keysUpTo(100).slice(3));

    act(() => screen.getByRole("button", { name: "Stick" }).focus());
    expect(renderedKeys()).toEqual(keysUpTo(100).slice(90));
  });

  it("releases the focused row once the focus is gone without a blur", () => {
    const { scroller } = renderLaidOut(keysUpTo(100));
    scrollTo(scroller, 0);
    act(() => screen.getByRole("button", { name: "r1" }).focus());
    vi.spyOn(document, "activeElement", "get").mockReturnValue(document.body);
    scrollTo(scroller, 100 * ROW_HEIGHT - VIEWPORT);
    expect(renderedKeys()).toEqual(keysUpTo(100).slice(90));
  });
});
