import {
  KeyboardEvent,
  RefObject,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ChatEntry } from "./chatEntries";
import { conversationTurns, turnAt } from "./conversationTurns";

const MIN_TURNS = 2;
const JUMP_MARGIN_PX = 16;

interface Timeline {
  scrollRef: RefObject<HTMLDivElement | null>;
  scrollTo: (top: number) => void;
  scrollToRow: (key: string) => void;
}

const renderedRows = (scroller: HTMLElement) => [
  ...scroller.querySelectorAll<HTMLElement>("[data-row-key]"),
];

function firstVisibleRowKey(scroller: HTMLElement) {
  const readingLine = scroller.getBoundingClientRect().top + JUMP_MARGIN_PX;
  const row = renderedRows(scroller).find(
    (row) => row.getBoundingClientRect().bottom > readingLine
  );
  return row?.dataset.rowKey ?? null;
}

const markCenter = (mark: HTMLElement) => mark.offsetTop + mark.offsetHeight / 2;

const singleLine = (text: string) => text.replace(/\s+/g, " ").trim();

const NAVIGATION_KEYS: Record<string, (index: number, count: number) => number> = {
  ArrowUp: (index) => Math.max(0, index - 1),
  ArrowDown: (index, count) => Math.min(count - 1, index + 1),
  Home: () => 0,
  End: (_, count) => count - 1,
};

/**
 * A mark per user message beside the conversation: hovering or focusing one previews the message
 * and its reply, and clicking it scrolls the message to the top, mounting it first if needed.
 */
export function ConversationMinimap({
  rows,
  timeline,
}: {
  rows: readonly ChatEntry[];
  timeline: Timeline;
}) {
  const { scrollRef, scrollTo, scrollToRow } = timeline;
  const turns = useMemo(() => conversationTurns(rows), [rows]);
  const marks = useRef<(HTMLButtonElement | null)[]>([]);
  const [visibleRow, setVisibleRow] = useState<string | null>(null);
  const [previewed, setPreviewed] = useState<{ index: number; top: number } | null>(null);
  const [jumpTarget, setJumpTarget] = useState<string | null>(null);
  const shown = turns.length >= MIN_TURNS;
  const tooltipId = useId();

  useEffect(() => {
    const scroller = scrollRef.current;
    if (!shown || !scroller) return;
    const update = () => setVisibleRow(firstVisibleRowKey(scroller));
    update();
    scroller.addEventListener("scroll", update);
    return () => scroller.removeEventListener("scroll", update);
  }, [scrollRef, shown]);

  useLayoutEffect(() => {
    const scroller = scrollRef.current;
    if (jumpTarget === null || !scroller) return;
    setJumpTarget(null);
    const row = renderedRows(scroller).find((row) => row.dataset.rowKey === jumpTarget);
    if (!row) return;
    const offset = row.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
    scrollTo(scroller.scrollTop + offset - JUMP_MARGIN_PX);
  }, [jumpTarget, scrollRef, scrollTo]);

  if (!shown) return null;

  const visibleIndex = rows.findIndex((row) => row.id === visibleRow);
  const current = visibleIndex < 0 ? turns.length - 1 : Math.max(0, turnAt(turns, visibleIndex));

  const jump = (rowKey: string) => {
    setJumpTarget(rowKey);
    scrollToRow(rowKey);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape" && previewed) {
      event.preventDefault();
      return setPreviewed(null);
    }
    const target = NAVIGATION_KEYS[event.key];
    const focused = marks.current.indexOf(document.activeElement as HTMLButtonElement);
    if (!target || focused < 0) return;
    event.preventDefault();
    marks.current[target(focused, turns.length)]?.focus();
  };

  const preview = previewed && turns[previewed.index];
  const showPreview = (index: number, mark: HTMLElement) =>
    setPreviewed({ index, top: markCenter(mark) });

  return (
    <div className="pointer-events-none sticky top-0 z-10 h-0 -mb-3">
      <nav
        aria-label="Conversation messages"
        onKeyDown={onKeyDown}
        onMouseLeave={() => setPreviewed(null)}
        onBlur={() => setPreviewed(null)}
        className="pointer-events-auto absolute right-full top-[50cqh] -translate-y-1/2 max-h-[60cqh] flex flex-col"
      >
        <div className="min-h-0 flex flex-col overflow-hidden">
          {turns.map((turn, index) => (
            <button
              key={turn.rowKey}
              ref={(mark) => {
                marks.current[index] = mark;
              }}
              type="button"
              tabIndex={index === current ? 0 : -1}
              aria-label={`Jump to message ${index + 1}`}
              aria-current={index === current ? "location" : undefined}
              aria-describedby={index === previewed?.index ? tooltipId : undefined}
              onClick={() => jump(turn.rowKey)}
              onMouseEnter={(event) => showPreview(index, event.currentTarget)}
              onFocus={(event) => showPreview(index, event.currentTarget)}
              className="group h-2 min-h-0 w-4 shrink flex items-center cursor-pointer outline-none"
            >
              <span
                className={`h-0.5 rounded-full transition-all group-hover:w-3.5 group-hover:bg-text-primary group-focus-visible:w-3.5 group-focus-visible:bg-accent ${index === current ? "w-3.5 bg-text-secondary" : "w-2 bg-text-muted"}`}
              />
            </button>
          ))}
        </div>
        {preview && (
          <div
            id={tooltipId}
            role="tooltip"
            style={{ top: previewed.top }}
            className="pointer-events-none absolute left-full ml-1 z-20 w-72 -translate-y-1/2 rounded-lg border border-border bg-bg-secondary p-2 text-xs shadow-lg"
          >
            <p className="truncate font-medium text-text-primary">{singleLine(preview.prompt)}</p>
            {preview.reply && (
              <p className="mt-1 line-clamp-3 text-text-secondary">{singleLine(preview.reply)}</p>
            )}
          </div>
        )}
      </nav>
    </div>
  );
}
