import { useEffect, useRef } from "react";
import { CheckIcon } from "../ui/Icons";

export interface MenuOption {
  id: string;
  label: string;
  description?: string;
  hint?: string;
  selected?: boolean;
}

interface OptionMenuProps {
  label: string;
  options: MenuOption[];
  active: number;
  numbered?: boolean;
  empty?: string;
  onPick: (option: MenuOption) => void;
  onHover: (index: number) => void;
}

/** Listbox shown above the composer; keyboard navigation is driven by the composer's textarea. */
export function OptionMenu({
  label,
  options,
  active,
  numbered,
  empty,
  onPick,
  onHover,
}: OptionMenuProps) {
  const listRef = useRef<HTMLUListElement>(null);

  useEffect(() => {
    listRef.current
      ?.querySelector<HTMLElement>(`[data-index="${active}"]`)
      ?.scrollIntoView?.({ block: "nearest" });
  }, [active]);

  return (
    <div className="absolute bottom-full left-0 right-0 mb-2 rounded-lg border border-border bg-bg-secondary shadow-xl overflow-hidden z-10">
      <div className="px-3 py-1.5 text-[0.6875rem] uppercase tracking-wide text-text-muted border-b border-border">
        {label}
      </div>
      {options.length === 0 ? (
        <p className="px-3 py-2 text-xs text-text-muted">{empty ?? "No matches"}</p>
      ) : (
        <ul
          ref={listRef}
          role="listbox"
          aria-label={label}
          className="max-h-64 overflow-y-auto py-1"
        >
          {options.map((option, index) => (
            <li
              key={option.id}
              role="option"
              data-index={index}
              aria-selected={index === active}
              onMouseEnter={() => onHover(index)}
              onMouseDown={(e) => {
                e.preventDefault();
                onPick(option);
              }}
              className={`flex items-baseline gap-2 px-3 py-1.5 text-xs cursor-pointer ${
                index === active ? "bg-bg-hover" : ""
              }`}
            >
              {numbered && index < 9 && (
                <kbd className="w-3 text-[0.625rem] font-mono text-text-muted">{index + 1}</kbd>
              )}
              <span className="font-medium text-text-primary whitespace-nowrap">
                {option.label}
              </span>
              {option.hint && (
                <span className="font-mono text-text-muted whitespace-nowrap">{option.hint}</span>
              )}
              {option.description && (
                <span className="flex-1 min-w-0 truncate text-text-muted">
                  {option.description}
                </span>
              )}
              {option.selected && <CheckIcon className="ml-auto text-accent flex-shrink-0" />}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
