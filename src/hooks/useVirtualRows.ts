import {
  FocusEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { observeResize } from "../observeResize";

const ESTIMATED_ROW_HEIGHT = 120;
const INITIAL_ROWS = 30;
const AT_BOTTOM_TOLERANCE_PX = 2;
const MIN_CORRECTION_PX = 1;

/** The viewport starts `fromKey` pixels past where row `key` starts, so rows inserted or resized above that row move it along. */
interface Viewport {
  key: string | undefined;
  fromKey: number;
  height: number;
}

interface VisibleRow {
  key: string;
  offset: number;
  element: Element;
}

/** The row the view stays on, with how much content sits below its top. Only content below it changing moves it on screen, and this distance changes with it but not with scrolling. */
interface Anchor {
  key: string;
  below: number;
}

export interface RowSpan {
  start: number;
  end: number;
}

/** The rows that overlap `[top, bottom)`, where `offsets[i]` is where row `i` starts and the last offset is the total height. */
export function rowsBetween(offsets: readonly number[], top: number, bottom: number): RowSpan {
  const count = offsets.length - 1;
  let start = 0;
  while (start < count && offsets[start + 1] <= top) start++;
  let end = start;
  while (end < count && offsets[end] < bottom) end++;
  return { start, end };
}

function rowOffsets(keys: readonly string[], heights: ReadonlyMap<string, number>) {
  const offsets = [0];
  for (const key of keys) {
    offsets.push(offsets[offsets.length - 1] + (heights.get(key) ?? ESTIMATED_ROW_HEIGHT));
  }
  return offsets;
}

function viewportTop(offsets: readonly number[], keys: readonly string[], viewport: Viewport) {
  const index = viewport.key === undefined ? -1 : keys.indexOf(viewport.key);
  return (index > 0 ? offsets[index] : 0) + viewport.fromKey;
}

function visibleSpan(
  offsets: readonly number[],
  keys: readonly string[],
  viewport: Viewport | null,
  stuck: boolean
) {
  const count = offsets.length - 1;
  if (!viewport) return { start: Math.max(0, count - INITIAL_ROWS), end: count };
  const total = offsets[count];
  const top = stuck ? total - viewport.height : viewportTop(offsets, keys, viewport);
  return rowsBetween(offsets, top - viewport.height, top + viewport.height * 2);
}

function includePins(span: RowSpan, keys: readonly string[], pins: readonly (string | null)[]) {
  let { start, end } = span;
  for (const pin of pins) {
    const index = pin === null ? -1 : keys.indexOf(pin);
    if (index < 0) continue;
    start = Math.min(start, index);
    end = Math.max(end, index + 1);
  }
  return { start, end };
}

/** The scroller lays its content out from the bottom, so `scrollTop` runs from minus the overflow up to 0 at the bottom; this is the same position counted from the top of the content. */
const scrolledFromTop = (scroller: HTMLElement) =>
  scroller.scrollHeight - scroller.clientHeight + scroller.scrollTop;

const scrollTopAt = (scroller: HTMLElement, fromTop: number) =>
  fromTop - (scroller.scrollHeight - scroller.clientHeight);

const sectionTopIn = (scroller: HTMLElement, section: HTMLElement | null) =>
  section
    ? section.getBoundingClientRect().top -
      scroller.getBoundingClientRect().top +
      scrolledFromTop(scroller)
    : 0;

const sameKeys = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((key, index) => key === b[index]);

/**
 * Renders only the rows near the viewport of a scroller that lays its content out from the bottom
 * (`flex-direction: column-reverse`). The browser keeps the bottom in place, so the scroller
 * follows new rows while it is at the bottom, and rows above the viewport can mount, change height
 * or be prepended without moving what is on screen. When content below the first visible row
 * changes instead, the row is put back where it was. That row, and rows holding the text selection
 * or the focus, stay mounted. The viewport is measured only when the user scrolls or the scroller
 * resizes, never in response to the hook's own renders; rows that change above it carry it along.
 */
export function useVirtualRows(keys: readonly string[]) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const sectionRef = useRef<HTMLDivElement>(null);
  const keysRef = useRef(keys);
  const offsetsRef = useRef<readonly number[]>([0]);
  const rowElements = useRef(new Map<string, HTMLElement>());
  const anchorRef = useRef<Anchor | null>(null);
  const stuckRef = useRef(true);
  const [stuck, setStuck] = useState(true);
  const [heights, setHeights] = useState<ReadonlyMap<string, number>>(() => new Map());
  const [viewport, setViewport] = useState<Viewport | null>(null);
  const [selectionKeys, setSelectionKeys] = useState<readonly string[]>([]);
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const [revealedKey, setRevealedKey] = useState<string | null>(null);
  const [anchorKey, setAnchorKey] = useState<string | null>(null);

  const offsets = useMemo(() => rowOffsets(keys, heights), [keys, heights]);
  const { start, end } = includePins(visibleSpan(offsets, keys, viewport, stuck), keys, [
    ...selectionKeys,
    focusKey,
    revealedKey,
    anchorKey,
  ]);

  const changeStuck = useCallback((value: boolean) => {
    if (stuckRef.current === value) return;
    stuckRef.current = value;
    setStuck(value);
  }, []);

  const contentBelow = useCallback(
    (row: Element) =>
      scrollRef.current!.firstElementChild!.getBoundingClientRect().bottom -
      row.getBoundingClientRect().top,
    []
  );

  /** Anchors the view to `row` and keeps it mounted until the next scroll picks another. */
  const changeAnchor = useCallback(
    (row: VisibleRow | null) => {
      const key = row?.key ?? null;
      if (key === null && anchorRef.current === null) return;
      anchorRef.current = row && { key: row.key, below: contentBelow(row.element) };
      setAnchorKey(key);
    },
    [contentBelow]
  );

  const firstVisibleRow = useCallback((): VisibleRow | null => {
    const scroller = scrollRef.current!;
    const viewportTop = scroller.getBoundingClientRect().top;
    const viewportBottom = viewportTop + scroller.clientHeight;
    for (const row of sectionRef.current?.children ?? []) {
      const key = (row as HTMLElement).dataset.rowKey;
      const { top, bottom } = row.getBoundingClientRect();
      if (key && bottom > viewportTop && top < viewportBottom) {
        return { key, offset: top - viewportTop, element: row };
      }
    }
    return null;
  }, []);

  /** Where the viewport starts, relative to the first visible row so rows that mounted but are not measured yet do not shift it; with no row in view, relative to the first row. */
  const scrolledPastRow = useCallback(
    (scroller: HTMLElement) => {
      const visible = firstVisibleRow();
      const index = visible ? keysRef.current.indexOf(visible.key) : -1;
      return visible && index >= 0
        ? {
            key: visible.key,
            rowTop: offsetsRef.current[index],
            top: offsetsRef.current[index] - visible.offset,
          }
        : {
            key: keysRef.current[0],
            rowTop: 0,
            top: scrolledFromTop(scroller) - sectionTopIn(scroller, sectionRef.current),
          };
    },
    [firstVisibleRow]
  );

  const measureViewport = useCallback(() => {
    const scroller = scrollRef.current;
    if (!scroller?.clientHeight) return;
    const height = scroller.clientHeight;
    const step = height / 2;
    const { key, rowTop, top } = scrolledPastRow(scroller);
    const steppedTop = Math.floor(top / step) * step;
    const fromKey = steppedTop - rowTop;
    setViewport((current) =>
      current?.height === height && current.key === key && current.fromKey === fromKey
        ? current
        : { key, fromKey, height }
    );
  }, [scrolledPastRow]);

  /** Puts the anchored row back where it was when the content below it changed height. */
  const keepPosition = useCallback(() => {
    const scroller = scrollRef.current!;
    if (!scroller.clientHeight || stuckRef.current) return;
    const anchor = anchorRef.current;
    const row = anchor && rowElements.current.get(anchor.key);
    if (!anchor || !row) {
      changeAnchor(firstVisibleRow());
      measureViewport();
      return;
    }
    const below = contentBelow(row);
    if (Math.abs(below - anchor.below) < MIN_CORRECTION_PX) return;
    scroller.scrollTop -= below - anchor.below;
    anchorRef.current = { key: anchor.key, below };
  }, [firstVisibleRow, changeAnchor, contentBelow, measureViewport]);

  const onScroll = () => {
    const scroller = scrollRef.current!;
    if (!scroller.clientHeight) return;
    const atBottom = scroller.scrollTop > -AT_BOTTOM_TOLERANCE_PX;
    changeStuck(atBottom);
    if (focusKey !== null && !sectionRef.current?.contains(document.activeElement)) {
      setFocusKey(null);
    }
    changeAnchor(atBottom ? null : firstVisibleRow());
    measureViewport();
  };

  const stickToBottom = useCallback(() => {
    changeAnchor(null);
    changeStuck(true);
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [changeAnchor, changeStuck]);

  const scrollTo = useCallback(
    (top: number) => {
      changeAnchor(null);
      changeStuck(false);
      scrollRef.current!.scrollTop = top;
      measureViewport();
    },
    [changeAnchor, changeStuck, measureViewport]
  );

  /** Scrolls to where the row would start when it is not rendered, so the next render mounts it. */
  const scrollToRow = useCallback(
    (key: string) => {
      const index = keysRef.current.indexOf(key);
      if (index < 0 || rowElements.current.has(key)) return;
      const scroller = scrollRef.current!;
      const rowTop = sectionTopIn(scroller, sectionRef.current) + offsetsRef.current[index];
      scrollTo(scrollTopAt(scroller, rowTop));
    },
    [scrollTo]
  );

  /** Keeps the row mounted and scrolls to it if it is not; `null` lets it unmount again. */
  const reveal = useCallback(
    (key: string | null) => {
      setRevealedKey(key);
      if (key !== null) scrollToRow(key);
    },
    [scrollToRow]
  );

  const rowRef = useCallback(
    (row: HTMLElement | null) => {
      if (!row) return;
      const key = row.dataset.rowKey!;
      rowElements.current.set(key, row);
      const stop = observeResize(row, () => {
        const height = row.getBoundingClientRect().height;
        if (height > 0) {
          setHeights((current) =>
            current.get(key) === height ? current : new Map(current).set(key, height)
          );
        }
        keepPosition();
      });
      return () => {
        stop();
        rowElements.current.delete(key);
      };
    },
    [keepPosition]
  );

  const rowKeyAt = useCallback((node: Node, offset = 0) => {
    const sectionRange = document.createRange();
    sectionRange.selectNodeContents(sectionRef.current!);
    const position = sectionRange.comparePoint(node, offset);
    if (position < 0) return keysRef.current[0] ?? null;
    if (position > 0) return keysRef.current.at(-1) ?? null;
    const element = node instanceof Element ? node : node.parentElement;
    return element?.closest<HTMLElement>("[data-row-key]")?.dataset.rowKey ?? null;
  }, []);

  const onFocus = (event: FocusEvent) => setFocusKey(rowKeyAt(event.target));

  const onBlur = (event: FocusEvent) => {
    if (!sectionRef.current?.contains(event.relatedTarget as Node | null)) setFocusKey(null);
  };

  useLayoutEffect(() => {
    keysRef.current = keys;
    offsetsRef.current = offsets;
  }, [keys, offsets]);

  useLayoutEffect(keepPosition);

  useLayoutEffect(() => {
    const scroller = scrollRef.current!;
    const onResize = () => {
      measureViewport();
      keepPosition();
    };
    const stops = [
      observeResize(scroller, onResize),
      observeResize(scroller.firstElementChild!, keepPosition),
    ];
    return () => stops.forEach((stop) => stop());
  }, [measureViewport, keepPosition]);

  useEffect(() => {
    const onSelectionChange = () => {
      const selection = document.getSelection();
      const section = sectionRef.current;
      const next =
        selection &&
        !selection.isCollapsed &&
        section &&
        selection.getRangeAt(0).intersectsNode(section)
          ? [
              rowKeyAt(selection.anchorNode!, selection.anchorOffset),
              rowKeyAt(selection.focusNode!, selection.focusOffset),
            ].filter((key): key is string => key !== null)
          : [];
      setSelectionKeys((current) => (sameKeys(current, next) ? current : next));
    };
    document.addEventListener("selectionchange", onSelectionChange);
    return () => document.removeEventListener("selectionchange", onSelectionChange);
  }, [rowKeyAt]);

  return {
    scrollRef,
    sectionRef,
    rowRef,
    start,
    end,
    nearTop: viewport !== null && start === 0,
    before: offsets[start],
    after: offsets[keys.length] - offsets[end],
    onScroll,
    onFocus,
    onBlur,
    stickToBottom,
    scrollTo,
    scrollToRow,
    reveal,
  };
}
