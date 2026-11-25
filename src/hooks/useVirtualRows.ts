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
const STICK_TO_BOTTOM_THRESHOLD_PX = 40;

interface Viewport {
  top: number;
  height: number;
}

interface Anchor {
  key: string;
  offset: number;
  scrollTop: number;
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

function visibleSpan(offsets: readonly number[], viewport: Viewport | null, stuck: boolean) {
  const count = offsets.length - 1;
  if (!viewport) return { start: Math.max(0, count - INITIAL_ROWS), end: count };
  const total = offsets[count];
  const top = stuck ? total - viewport.height : viewport.top;
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

const sectionTopIn = (scroller: HTMLElement, section: HTMLElement | null) =>
  section ? section.offsetTop - scroller.offsetTop : 0;

const sameKeys = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((key, index) => key === b[index]);

/**
 * Renders only the rows near the viewport of a bottom-anchored scroller. Rows are measured as they
 * mount; the scroller stays at the bottom while it is there, and otherwise keeps the first visible
 * row in place when rows above it mount or change height. Rows holding the text selection or the
 * focus stay mounted.
 */
export function useVirtualRows(keys: readonly string[]) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const sectionRef = useRef<HTMLDivElement>(null);
  const keysRef = useRef(keys);
  const offsetsRef = useRef<readonly number[]>([0]);
  const lastScrollTopRef = useRef(0);
  const rowElements = useRef(new Map<string, HTMLElement>());
  const anchorRef = useRef<Anchor | null>(null);
  const stuckRef = useRef(true);
  const [stuck, setStuck] = useState(true);
  const [heights, setHeights] = useState<ReadonlyMap<string, number>>(() => new Map());
  const [viewport, setViewport] = useState<Viewport | null>(null);
  const [selectionKeys, setSelectionKeys] = useState<readonly string[]>([]);
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const [revealedKey, setRevealedKey] = useState<string | null>(null);

  const offsets = useMemo(() => rowOffsets(keys, heights), [keys, heights]);
  const { start, end } = includePins(visibleSpan(offsets, viewport, stuck), keys, [
    ...selectionKeys,
    focusKey,
    revealedKey,
  ]);

  const changeStuck = useCallback((value: boolean) => {
    if (stuckRef.current === value) return;
    stuckRef.current = value;
    setStuck(value);
  }, []);

  const firstVisibleRow = useCallback((): Anchor | null => {
    const scroller = scrollRef.current!;
    const viewportTop = scroller.getBoundingClientRect().top;
    const viewportBottom = viewportTop + scroller.clientHeight;
    for (const row of sectionRef.current?.children ?? []) {
      const key = (row as HTMLElement).dataset.rowKey;
      const { top, bottom } = row.getBoundingClientRect();
      if (key && bottom > viewportTop && top < viewportBottom) {
        return { key, offset: top - viewportTop, scrollTop: scroller.scrollTop };
      }
    }
    return null;
  }, []);

  const measureViewport = useCallback(() => {
    const scroller = scrollRef.current;
    const section = sectionRef.current;
    if (!scroller?.clientHeight) return;
    const height = scroller.clientHeight;
    const sectionTop = sectionTopIn(scroller, section);
    const step = height / 2;
    const top = Math.floor((scroller.scrollTop - sectionTop) / step) * step;
    setViewport((current) =>
      current?.top === top && current.height === height ? current : { top, height }
    );
  }, []);

  const keepPosition = useCallback(() => {
    const scroller = scrollRef.current!;
    if (!scroller.clientHeight) return;
    if (stuckRef.current) {
      scroller.scrollTop = scroller.scrollHeight;
      lastScrollTopRef.current = scroller.scrollTop;
      return;
    }
    const anchor = anchorRef.current;
    const index = anchor ? keysRef.current.indexOf(anchor.key) : -1;
    if (!anchor || index < 0 || anchor.scrollTop !== scroller.scrollTop) {
      anchorRef.current = firstVisibleRow();
      return;
    }
    const row = rowElements.current.get(anchor.key);
    if (row) {
      const offset = row.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
      if (offset !== anchor.offset) scroller.scrollTop += offset - anchor.offset;
    } else {
      const rowTop = sectionTopIn(scroller, sectionRef.current) + offsetsRef.current[index];
      scroller.scrollTop = rowTop - anchor.offset;
      measureViewport();
    }
    anchor.scrollTop = scroller.scrollTop;
  }, [firstVisibleRow, measureViewport]);

  const onScroll = () => {
    const scroller = scrollRef.current!;
    if (!scroller.clientHeight) return;
    const movedUp = scroller.scrollTop < lastScrollTopRef.current;
    lastScrollTopRef.current = scroller.scrollTop;
    const distanceFromBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
    const clampedToBottom = distanceFromBottom < 1;
    const nearBottom = distanceFromBottom < STICK_TO_BOTTOM_THRESHOLD_PX;
    const atBottom = clampedToBottom || (!movedUp && (nearBottom || stuckRef.current));
    changeStuck(atBottom);
    if (focusKey !== null && !sectionRef.current?.contains(document.activeElement)) {
      setFocusKey(null);
    }
    anchorRef.current = atBottom ? null : firstVisibleRow();
    measureViewport();
    if (atBottom) keepPosition();
  };

  const stickToBottom = useCallback(() => {
    anchorRef.current = null;
    changeStuck(true);
  }, [changeStuck]);

  const scrollTo = useCallback(
    (top: number) => {
      anchorRef.current = null;
      changeStuck(false);
      scrollRef.current!.scrollTop = top;
      measureViewport();
    },
    [changeStuck, measureViewport]
  );

  /** Scrolls to where the row would start when it is not rendered, so the next render mounts it. */
  const scrollToRow = useCallback(
    (key: string) => {
      const index = keysRef.current.indexOf(key);
      if (index < 0 || rowElements.current.has(key)) return;
      scrollTo(sectionTopIn(scrollRef.current!, sectionRef.current) + offsetsRef.current[index]);
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
        const height = row.offsetHeight;
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
