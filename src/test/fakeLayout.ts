import { vi } from "vitest";

const styleHeight = (element: HTMLElement) => parseFloat(element.style.height) || 0;

/**
 * Gives a virtualized scroller a jsdom layout: a `viewport`-tall scroller that lays its content out
 * from the bottom, like `flex-direction: column-reverse`, whose rows (elements with `data-row-key`)
 * are `rowHeight(key)` tall and whose spacers take their inline height. `offsetHeight` rounds, as
 * browsers do. `scrollTop` is 0 at the
 * bottom and negative above it, and it keeps its distance from the bottom when the content changes,
 * as browsers do for such a scroller.
 */
export function layOutScroller(
  scroller: HTMLElement,
  viewport: number,
  rowHeight: (key: string) => number
) {
  const heightOf = (element: HTMLElement) =>
    element.dataset.rowKey ? rowHeight(element.dataset.rowKey) : styleHeight(element);
  const section = () => scroller.querySelector("[data-row-key]")?.parentElement ?? null;
  const blocks = () => [...(section()?.children ?? [])];
  const contentHeight = () =>
    blocks().reduce((sum, block) => sum + heightOf(block as HTMLElement), 0);
  const overflow = () => Math.max(0, contentHeight() - viewport);
  let scrollTop = 0;
  const currentScrollTop = () => Math.max(scrollTop, -overflow()) || 0;
  const visibleTop = () => overflow() + currentScrollTop();
  Object.defineProperties(scroller, {
    clientHeight: { configurable: true, get: () => viewport },
    scrollHeight: { configurable: true, get: () => Math.max(viewport, contentHeight()) },
    scrollTop: {
      configurable: true,
      get: currentScrollTop,
      set: (value: number) => {
        scrollTop = Math.min(0, Math.max(value, -overflow())) || 0;
      },
    },
  });
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (
    this: HTMLElement
  ) {
    return Math.round(heightOf(this));
  });
  const rect = (top: number, height: number) =>
    ({ top, bottom: top + height, height, left: 0, right: 0, width: 0, x: 0, y: top }) as DOMRect;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement
  ) {
    if (this === scroller) return rect(0, viewport);
    if (this === section() || this === scroller.firstElementChild) {
      return rect(-visibleTop(), contentHeight());
    }
    if (this.parentElement !== section()) return rect(0, 0);
    let top = -visibleTop();
    for (const block of blocks()) {
      if (block === this) break;
      top += heightOf(block as HTMLElement);
    }
    return rect(top, heightOf(this));
  });
}

/** How far the scroller laid out by `layOutScroller` is from the top of its content. */
export const positionOf = (scroller: HTMLElement) =>
  scroller.scrollHeight - scroller.clientHeight + scroller.scrollTop;

/** Scrolls the scroller laid out by `layOutScroller` to `top` pixels from the top of its content. */
export const placeAt = (scroller: HTMLElement, top: number) => {
  scroller.scrollTop = top - (scroller.scrollHeight - scroller.clientHeight);
};
