import { ReactNode } from "react";

export function CollapsibleSectionHeader({
  expanded,
  onToggle,
  controls,
  disabled,
  children,
}: {
  expanded: boolean;
  onToggle: () => void;
  controls: string;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-expanded={expanded}
      aria-controls={controls}
      className="w-full flex items-center gap-2 py-2 text-left text-sm font-semibold text-text-primary cursor-pointer focus-visible:outline-accent"
      disabled={disabled}
      onClick={onToggle}
    >
      <span aria-hidden="true" className="text-text-muted">
        {expanded ? "▾" : "▸"}
      </span>
      {children}
    </button>
  );
}
