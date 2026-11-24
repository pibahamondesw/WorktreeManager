import { KeyboardEvent, useEffect, useRef } from "react";
import { ChevronDownIcon, ChevronUpIcon, CloseIcon } from "../ui/Icons";
import { ChatFind } from "./useChatFind";

function resultLabel({ query, position, total, searchingEarlier }: ChatFind) {
  if (!query) return "";
  if (total === 0) return searchingEarlier ? "Searching…" : "No results";
  return `${position}/${total}${searchingEarlier ? "…" : ""}`;
}

const iconButton =
  "h-6 w-6 flex items-center justify-center rounded text-text-muted hover:bg-bg-hover hover:text-text-primary disabled:opacity-40 disabled:pointer-events-none cursor-pointer";

export function ChatFindBar({ find }: { find: ChatFind }) {
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [find.focusRequest]);
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Enter") {
      event.preventDefault();
      if (event.shiftKey) find.next();
      else find.previous();
    } else if (event.key === "Escape") {
      event.preventDefault();
      find.close();
    }
  };
  const empty = find.total === 0;
  return (
    <div
      role="search"
      aria-label="Find in conversation"
      className="shrink-0 flex items-center justify-end gap-1 border-b border-border px-3 py-1"
    >
      <input
        ref={inputRef}
        type="text"
        value={find.query}
        onChange={(event) => find.changeQuery(event.target.value)}
        onKeyDown={onKeyDown}
        placeholder="Find in conversation"
        aria-label="Find in conversation"
        className="w-56 rounded border border-border bg-bg-tertiary px-2 py-0.5 text-xs text-text-primary placeholder:text-text-muted outline-none focus:border-accent"
      />
      <span role="status" className="min-w-16 text-center text-xs text-text-muted tabular-nums">
        {resultLabel(find)}
      </span>
      <button
        type="button"
        onClick={find.previous}
        disabled={empty}
        title="Previous match (↵)"
        aria-label="Previous match"
        className={iconButton}
      >
        <ChevronUpIcon size={14} />
      </button>
      <button
        type="button"
        onClick={find.next}
        disabled={empty}
        title="Next match (⇧↵)"
        aria-label="Next match"
        className={iconButton}
      >
        <ChevronDownIcon size={14} />
      </button>
      <button
        type="button"
        onClick={find.close}
        title="Close find (Esc)"
        aria-label="Close find"
        className={iconButton}
      >
        <CloseIcon size={14} />
      </button>
    </div>
  );
}
