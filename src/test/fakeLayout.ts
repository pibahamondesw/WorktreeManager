import { vi } from "vitest";

const styleHeight = (element: HTMLElement) => parseFloat(element.style.height) || 0;

/**
 * Gives a virtualized scroller a jsdom layout: a `viewport`-tall scroller whose rows (elements with
 * `data-row-key`) are `rowHeight(key)` tall and whose spacers take their inline height.
 */
export function layOutScroller(
  scroller: HTMLElement,
  viewport: number,
  rowHeight: (key: string) => number
) {
  const heightOf = (element: HTMLElement) =>
    element.dataset.rowKey ? rowHeight(element.dataset.rowKey) : styleHeight(element);
  const blocks = () => [
    ...(scroller.querySelector("[data-row-key]")?.parentElement?.children ?? []),
  ];
  const contentHeight = () =>
    blocks().reduce((sum, block) => sum + heightOf(block as HTMLElement), 0);
  let scrollTop = 0;
  Object.defineProperties(scroller, {
    clientHeight: { configurable: true, get: () => viewport },
    scrollHeight: { configurable: true, get: () => Math.max(viewport, contentHeight()) },
    scrollTop: {
      configurable: true,
      get: () => scrollTop,
      set: (value: number) => {
        scrollTop = Math.max(0, Math.min(value, contentHeight() - viewport));
      },
    },
  });
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (
    this: HTMLElement
  ) {
    return heightOf(this);
  });
  const rect = (top: number, height: number) =>
    ({ top, bottom: top + height, height, left: 0, right: 0, width: 0, x: 0, y: top }) as DOMRect;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement
  ) {
    if (!this.dataset.rowKey) return rect(0, 0);
    let top = -scrollTop;
    for (const block of blocks()) {
      if (block === this) break;
      top += heightOf(block as HTMLElement);
    }
    return rect(top, heightOf(this));
  });
}
