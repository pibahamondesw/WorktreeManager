// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

vi.mock("../observeResize", () => import("../test/fakeObserveResize"));

import { rowsBetween, useVirtualRows } from "./useVirtualRows";
import { resize } from "../test/fakeObserveResize";
import { layOutScroller, placeAt, positionOf } from "../test/fakeLayout";

const ROW_HEIGHT = 120;
const VIEWPORT = 600;
const keysUpTo = (count: number) => Array.from({ length: count }, (_, index) => `r${index}`);

const nearTopRenders: boolean[] = [];

function Timeline({ keys }: { keys: string[] }) {
  const { scrollRef, sectionRef, rowRef, start, end, before, after, nearTop, ...handlers } =
    useVirtualRows(keys);
  nearTopRenders.push(nearTop);
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
  placeAt(scroller, top);
  fireEvent.scroll(scroller);
};

const rowTop = (key: string) =>
  document.querySelector(`[data-row-key="${key}"]`)!.getBoundingClientRect().top;

function recordWrites(scroller: HTMLElement) {
  const writes = vi.fn();
  const descriptor = Object.getOwnPropertyDescriptor(scroller, "scrollTop")!;
  Object.defineProperty(scroller, "scrollTop", {
    configurable: true,
    get: descriptor.get,
    set: (value: number) => {
      writes(value);
      descriptor.set!.call(scroller, value);
    },
  });
  return writes;
}

const resizeRow = (key: string) => resize([document.querySelector(`[data-row-key="${key}"]`)!]);

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
    expect(scroller.scrollTop).toBe(0);

    rerender(<Timeline keys={keysUpTo(101)} />);
    expect(screen.getByRole("button", { name: "r100" })).toBeInTheDocument();
    expect(scroller.scrollTop).toBe(0);
  });

  it("follows the bottom without writing the scroll position", () => {
    const heights: Record<string, number> = { r99: 420 };
    const { scroller, rerender } = renderLaidOut(
      keysUpTo(100),
      (key) => heights[key] ?? ROW_HEIGHT
    );
    const writes = recordWrites(scroller);

    heights.r99 = ROW_HEIGHT;
    resizeRow("r99");
    rerender(<Timeline keys={keysUpTo(101)} />);
    heights.r100 = 400;
    resizeRow("r100");

    expect(scroller.scrollTop).toBe(0);
    expect(writes).not.toHaveBeenCalled();
  });

  it("renders the rows around the viewport after scrolling up", () => {
    const { scroller, rerender } = renderLaidOut(keysUpTo(100));
    scrollTo(scroller, 0);
    expect(renderedKeys()).toEqual(keysUpTo(10));
    expect(screen.getByTestId("after")).toHaveStyle({ height: `${90 * ROW_HEIGHT}px` });

    rerender(<Timeline keys={keysUpTo(101)} />);
    expect(positionOf(scroller)).toBe(0);

    scrollTo(scroller, 6000);
    expect(renderedKeys()).toEqual(keysUpTo(100).slice(45, 60));

    fireEvent.click(screen.getByRole("button", { name: "Stick" }));
    expect(renderedKeys()).toEqual(keysUpTo(101).slice(91));
    expect(scroller.scrollTop).toBe(0);
  });

  it("lets a small scroll up leave the bottom and stop following new rows", () => {
    const { scroller, rerender } = renderLaidOut(keysUpTo(100));
    scrollTo(scroller, 100 * ROW_HEIGHT - VIEWPORT - 5);

    rerender(<Timeline keys={keysUpTo(101)} />);
    expect(positionOf(scroller)).toBe(100 * ROW_HEIGHT - VIEWPORT - 5);
  });

  it("follows new rows again after scrolling back down to the bottom", () => {
    const { scroller, rerender } = renderLaidOut(keysUpTo(100));
    scrollTo(scroller, 100 * ROW_HEIGHT - VIEWPORT - 100);

    scrollTo(scroller, 100 * ROW_HEIGHT - VIEWPORT);
    rerender(<Timeline keys={keysUpTo(101)} />);

    expect(scroller.scrollTop).toBe(0);
    expect(renderedKeys()).toContain("r100");
  });

  it("does not move or write the scroll position when a row above the viewport changes height", () => {
    const heights: Record<string, number> = {};
    const { scroller } = renderLaidOut(keysUpTo(100), (key) => heights[key] ?? ROW_HEIGHT);
    scrollTo(scroller, 6050);
    const writes = recordWrites(scroller);

    heights.r46 = 320;
    resizeRow("r46");
    heights.r47 = 20;
    resizeRow("r47");

    expect(rowTop("r50")).toBe(-50);
    expect(writes).not.toHaveBeenCalled();
  });

  it("keeps the view and writes nothing while scrolling up through rows shorter than estimated", () => {
    const { scroller } = renderLaidOut(keysUpTo(100), (key) =>
      Number(key.slice(1)) < 60 ? 20 : ROW_HEIGHT
    );
    scrollTo(scroller, 62 * ROW_HEIGHT);
    const writes = recordWrites(scroller);

    resize();

    expect(renderedKeys()).toContain("r55");
    expect(rowTop("r62")).toBe(0);
    expect(writes).not.toHaveBeenCalled();
  });

  it("keeps rendering the rows on screen when a row above the viewport grows a lot", () => {
    const heights: Record<string, number> = {};
    const { scroller } = renderLaidOut(keysUpTo(100), (key) => heights[key] ?? ROW_HEIGHT);
    scrollTo(scroller, 6050);

    heights.r46 = 1620;
    resizeRow("r46");

    expect(rowTop("r50")).toBe(-50);
    expect(renderedKeys()).toEqual(expect.arrayContaining(["r51", "r52", "r53", "r54"]));
  });

  it("keeps the first visible row in place when a row below it grows", () => {
    const heights: Record<string, number> = {};
    const { scroller } = renderLaidOut(keysUpTo(100), (key) => heights[key] ?? ROW_HEIGHT);
    scrollTo(scroller, 6050);

    heights.r52 = 320;
    resizeRow("r52");

    expect(rowTop("r50")).toBe(-50);
    expect(positionOf(scroller)).toBe(6050);
  });

  it("corrects content below that grows by less than a pixel only once it adds up to one", () => {
    const heights: Record<string, number> = {};
    const { scroller } = renderLaidOut(keysUpTo(100), (key) => heights[key] ?? ROW_HEIGHT);
    scrollTo(scroller, 6050);
    const writes = recordWrites(scroller);

    heights.r52 = ROW_HEIGHT + 0.5;
    resizeRow("r52");
    expect(writes).not.toHaveBeenCalled();

    heights.r52 = ROW_HEIGHT + 1;
    resizeRow("r52");
    expect(writes).toHaveBeenCalledTimes(1);
    expect(rowTop("r50")).toBe(-50);
  });

  it("writes nothing while scrolling to fractional positions over unchanged content", () => {
    const { scroller, rerender } = renderLaidOut(keysUpTo(100));
    scrollTo(scroller, 6050.4);
    placeAt(scroller, 6020.7);
    const writes = recordWrites(scroller);

    rerender(<Timeline keys={keysUpTo(100)} />);
    resize();

    expect(writes).not.toHaveBeenCalled();
  });

  it("writes nothing when rows of fractional height unmount below the viewport", () => {
    const { scroller } = renderLaidOut(keysUpTo(100), () => ROW_HEIGHT + 0.4);
    scrollTo(scroller, 11390);
    scrollTo(scroller, 10840);
    placeAt(scroller, 10290);
    const writes = recordWrites(scroller);

    fireEvent.scroll(scroller);
    resize();

    expect(renderedKeys()).not.toContain("r99");
    expect(writes).not.toHaveBeenCalled();
  });

  it("keeps the first visible row in place when older rows are prepended", () => {
    const keys = keysUpTo(100);
    const { scroller, rerender } = renderLaidOut(keys);
    scrollTo(scroller, 6050);
    const writes = recordWrites(scroller);

    const older = Array.from({ length: 50 }, (_, index) => `older${index}`);
    rerender(<Timeline keys={[...older, ...keys]} />);

    expect(rowTop("r50")).toBe(-50);
    expect(positionOf(scroller)).toBe(6050 + 50 * ROW_HEIGHT);
    expect(writes).not.toHaveBeenCalled();
  });

  it("keeps the first visible row in place when rows are inserted right above it", () => {
    const keys = keysUpTo(100);
    const { scroller, rerender } = renderLaidOut(keys);
    scrollTo(scroller, 6050);

    const inserted = Array.from({ length: 50 }, (_, index) => `inserted${index}`);
    rerender(<Timeline keys={[...keys.slice(0, 50), ...inserted, ...keys.slice(50)]} />);

    expect(rowTop("r50")).toBe(-50);
    expect(renderedKeys()).toEqual(expect.arrayContaining(["r51", "r52", "r53", "r54"]));
    expect(positionOf(scroller)).toBe(6050 + 50 * ROW_HEIGHT);
  });

  it("stops being near the top as soon as older rows are prepended", () => {
    const keys = keysUpTo(100);
    const { scroller, rerender } = renderLaidOut(keys);
    scrollTo(scroller, 0);
    expect(nearTopRenders.at(-1)).toBe(true);

    nearTopRenders.length = 0;
    const older = Array.from({ length: 50 }, (_, index) => `older${index}`);
    rerender(<Timeline keys={[...older, ...keys]} />);
    expect(nearTopRenders).not.toContain(true);
    expect(positionOf(scroller)).toBe(50 * ROW_HEIGHT);
  });

  it("keeps a scroll the scroller has not reported yet when a row below the first visible one grows", () => {
    const heights: Record<string, number> = {};
    const { scroller, rerender } = renderLaidOut(
      keysUpTo(100),
      (key) => heights[key] ?? ROW_HEIGHT
    );
    scrollTo(scroller, 6050);
    placeAt(scroller, 6100);
    rerender(<Timeline keys={keysUpTo(100)} />);
    expect(positionOf(scroller)).toBe(6100);

    heights.r52 = 320;
    resizeRow("r52");
    expect(rowTop("r50")).toBe(-100);
  });

  it("stays at the bottom when a row grows before its resize is reported", () => {
    const heights: Record<string, number> = {};
    const { scroller } = renderLaidOut(keysUpTo(100), (key) => heights[key] ?? ROW_HEIGHT);
    heights.r99 = 420;
    fireEvent.scroll(scroller);
    resizeRow("r99");
    expect(scroller.scrollTop).toBe(0);
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
