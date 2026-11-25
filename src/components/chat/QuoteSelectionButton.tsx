import { RefObject, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Button } from "../ui/Button";

interface QuotableSelection {
  itemId: string;
  text: string;
  top: number;
  left: number;
}

const BUTTON_HEIGHT_PX = 32;

function quotableSelection(
  section: HTMLElement | null,
  scroller: HTMLElement
): QuotableSelection | null {
  const selection = document.getSelection();
  if (!section || !selection || selection.isCollapsed) return null;
  const range = selection.getRangeAt(0);
  const common = range.commonAncestorContainer;
  const element = common instanceof Element ? common : common.parentElement;
  const source = element?.closest<HTMLElement>("[data-quote-source]");
  const text = selection.toString().trim();
  if (!source || !section.contains(source) || !text) return null;
  const rect = range.getBoundingClientRect();
  const view = scroller.getBoundingClientRect();
  if (rect.bottom < view.top || rect.top > view.bottom) return null;
  const above = rect.top - BUTTON_HEIGHT_PX;
  return {
    itemId: source.dataset.quoteSource!,
    text,
    top: above >= 0 ? above : rect.bottom,
    left: rect.left + rect.width / 2,
  };
}

/** Offers to quote the text selected in one of the agent's replies. */
export function QuoteSelectionButton({
  sectionRef,
  scrollRef,
  onQuote,
}: {
  sectionRef: RefObject<HTMLElement | null>;
  scrollRef: RefObject<HTMLElement | null>;
  onQuote: (itemId: string, text: string) => void;
}) {
  const [selection, setSelection] = useState<QuotableSelection | null>(null);

  useEffect(() => {
    const scroller = scrollRef.current!;
    const update = () => setSelection(quotableSelection(sectionRef.current, scroller));
    document.addEventListener("selectionchange", update);
    scroller.addEventListener("scroll", update, { passive: true });
    return () => {
      document.removeEventListener("selectionchange", update);
      scroller.removeEventListener("scroll", update);
    };
  }, [sectionRef, scrollRef]);

  if (!selection) return null;
  return createPortal(
    <Button
      variant="secondary"
      style={{ top: selection.top, left: selection.left }}
      className="fixed z-50 h-7 -translate-x-1/2 px-2 text-xs shadow-lg"
      onMouseDown={(event) => event.preventDefault()}
      onClick={() => {
        onQuote(selection.itemId, selection.text);
        document.getSelection()?.removeAllRanges();
        setSelection(null);
      }}
    >
      Quote in composer
    </Button>,
    document.body
  );
}
