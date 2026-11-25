import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ChatQuote } from "./chatQuotes";
import { textRanges } from "./chatFind";
import { scrollTopShowing, Timeline } from "./useChatFind";
import { RowSpan } from "../../hooks/useVirtualRows";

const QUOTE_HIGHLIGHT = "chat-quote";
const HIGHLIGHT_MS = 2000;

const highlightRegistry = () => (typeof CSS === "undefined" ? undefined : CSS.highlights);

/** Where the quote's text renders in `row`, falling back to its first line when the whole text spans blocks. */
export function quoteRange(row: Element, text: string) {
  const firstLine = text.split("\n")[0].trim();
  const roots = [...row.querySelectorAll("[data-find-text]")];
  for (const candidate of [text, firstLine]) {
    for (const root of roots) {
      const [range] = textRanges(root, candidate);
      if (range) return range;
    }
  }
  return null;
}

/** Mounts the quoted reply, scrolls to the quoted text and highlights it briefly. */
export function useQuoteNavigation(
  timeline: Pick<Timeline, "scrollRef" | "scrollTo"> &
    RowSpan & { scrollToRow: (key: string) => void }
) {
  const [target, setTarget] = useState<ChatQuote | null>(null);
  const highlightTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const { scrollRef, scrollTo, scrollToRow, start, end } = timeline;

  useLayoutEffect(() => {
    if (!target) return;
    const scroller = scrollRef.current!;
    const row = [...scroller.querySelectorAll<HTMLElement>("[data-row-key]")].find(
      (element) => element.dataset.rowKey === target.itemId
    );
    if (!row) return scrollToRow(target.itemId);
    const range = quoteRange(row, target.text);
    const top = scrollTopShowing(scroller, range ?? row);
    if (top !== null) scrollTo(top);
    const registry = highlightRegistry();
    if (range && registry) {
      registry.set(QUOTE_HIGHLIGHT, new Highlight(range));
      clearTimeout(highlightTimer.current);
      highlightTimer.current = setTimeout(() => registry.delete(QUOTE_HIGHLIGHT), HIGHLIGHT_MS);
    }
    setTarget(null);
  }, [target, scrollRef, scrollTo, scrollToRow, start, end]);

  useEffect(() => {
    const timer = highlightTimer;
    return () => {
      clearTimeout(timer.current);
      highlightRegistry()?.delete(QUOTE_HIGHLIGHT);
    };
  }, []);

  return setTarget;
}
