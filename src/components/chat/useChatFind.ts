import { RefObject, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useShortcutActions } from "../../shortcuts/runtime";
import { ChatEntry } from "./chatEntries";
import { FindMatch, findMatches, sameMatch, textRanges } from "./chatFind";

const MATCH_HIGHLIGHT = "chat-find";
const CURRENT_HIGHLIGHT = "chat-find-current";

export interface Timeline {
  scrollRef: RefObject<HTMLDivElement | null>;
  scrollTo: (top: number) => void;
  reveal: (key: string | null) => void;
}

function paintHighlights(matches: Range[], current: Range | undefined) {
  const registry = typeof CSS === "undefined" ? undefined : CSS.highlights;
  if (!registry) return;
  registry.set(MATCH_HIGHLIGHT, new Highlight(...matches));
  if (current) registry.set(CURRENT_HIGHLIGHT, new Highlight(current));
  else registry.delete(CURRENT_HIGHLIGHT);
}

const clearHighlights = () => paintHighlights([], undefined);

/** The scroll position that brings `target` to the upper third of `scroller`, or null when it is in view. */
export function scrollTopShowing(scroller: HTMLElement, target: Range | Element): number | null {
  const box = scroller.getBoundingClientRect();
  const rect = target.getBoundingClientRect();
  if (rect.top >= box.top && rect.bottom <= box.bottom) return null;
  return scroller.scrollTop + rect.top - box.top - scroller.clientHeight / 3;
}

/**
 * Finds text in the conversation: loads its earlier history while searching, mounts and scrolls
 * to the current match, and highlights the matches on screen. Starts at the newest match.
 */
export function useChatFind(
  rows: readonly ChatEntry[],
  timeline: Timeline,
  earlier: number,
  loadEarlier: () => void
) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<FindMatch | null>(null);
  const [focusRequest, setFocusRequest] = useState(0);
  const scrolledTo = useRef<FindMatch | null>(null);
  const painted = useRef(false);
  const returnFocus = useRef<HTMLElement | null>(null);
  const { scrollRef, scrollTo, reveal } = timeline;

  const matches = useMemo(() => (open ? findMatches(rows, query) : []), [open, rows, query]);
  const selectedIndex = matches.findIndex((match) => sameMatch(match, selected));
  const index = selectedIndex >= 0 ? selectedIndex : matches.length - 1;
  const current = matches[index] ?? null;
  const searchingEarlier = open && query !== "" && earlier > 0;

  useEffect(() => {
    if (searchingEarlier) loadEarlier();
  }, [searchingEarlier, earlier, loadEarlier]);

  useEffect(() => {
    if (current && !sameMatch(current, selected)) setSelected(current);
  }, [current, selected]);

  const currentRow = current?.rowKey ?? null;
  useEffect(() => reveal(currentRow), [currentRow, reveal]);

  useLayoutEffect(() => {
    const scroller = scrollRef.current;
    if (!open || !query || !scroller) {
      scrolledTo.current = null;
      if (painted.current) clearHighlights();
      painted.current = false;
      return;
    }
    const matchedRows = new Set(matches.map((match) => match.rowKey));
    const ranges: Range[] = [];
    let currentRange: Range | undefined;
    for (const row of scroller.querySelectorAll<HTMLElement>("[data-row-key]")) {
      const key = row.dataset.rowKey!;
      if (!matchedRows.has(key)) continue;
      const rowRanges = [...row.querySelectorAll("[data-find-text]")].flatMap((text) =>
        textRanges(text, query)
      );
      ranges.push(...rowRanges);
      if (key === current?.rowKey)
        currentRange = rowRanges[Math.min(current.occurrence, rowRanges.length - 1)];
    }
    paintHighlights(ranges, currentRange);
    painted.current = true;
    if (!currentRange || sameMatch(scrolledTo.current, current)) return;
    scrolledTo.current = current;
    const top = scrollTopShowing(scroller, currentRange);
    if (top !== null) scrollTo(top);
  });

  useEffect(() => clearHighlights, []);

  useShortcutActions({
    "conversation.find": {
      handler: () => {
        if (!open && document.activeElement instanceof HTMLElement)
          returnFocus.current = document.activeElement;
        setOpen(true);
        setFocusRequest((count) => count + 1);
      },
      inTextFields: true,
    },
  });

  const step = (delta: number) => {
    if (matches.length === 0) return;
    scrolledTo.current = null;
    setSelected(matches[(index + delta + matches.length) % matches.length]);
  };

  return {
    open,
    focusRequest,
    query,
    position: current ? index + 1 : 0,
    total: matches.length,
    searchingEarlier,
    changeQuery: (value: string) => {
      setQuery(value);
      setSelected(null);
    },
    previous: () => step(-1),
    next: () => step(1),
    close: () => {
      setOpen(false);
      if (returnFocus.current?.isConnected) returnFocus.current.focus();
      returnFocus.current = null;
    },
  };
}

export type ChatFind = ReturnType<typeof useChatFind>;
