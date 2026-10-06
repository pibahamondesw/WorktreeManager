import { useEffect, useRef, useState } from "react";
import { useClaudeUsage } from "../../hooks/useClaudeUsage";
import { useEditorOcclusion } from "../../hooks/useEditorOcclusion";
import { usageTone, UsageTone } from "../../services/usage";
import { CloseIcon } from "../ui/Icons";
import { PlanUsageContent } from "./PlanUsageContent";

const TONE: Record<UsageTone, string> = {
  normal: "text-text-secondary",
  warning: "text-warning",
  danger: "text-danger",
};

export function ClaudeUsageIndicator() {
  const { snapshot, error, loading, refresh } = useClaudeUsage();
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const windows = snapshot?.usage?.windows ?? [];
  const primaryWindows = windows.filter((window) => ["five_hour", "seven_day"].includes(window.id));
  const tone = usageTone(Math.max(0, ...windows.map((window) => window.usedPercent)));
  useEditorOcclusion(open);

  useEffect(() => {
    if (!open) return;
    void refresh();
    closeRef.current?.focus();
    const outside = (event: MouseEvent) => {
      if (event.target instanceof Node && !containerRef.current?.contains(event.target)) {
        setOpen(false);
      }
    };
    const keyboard = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
        triggerRef.current?.focus();
      } else if (event.key === "Tab") {
        event.preventDefault();
        closeRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", keyboard);
    return () => {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", keyboard);
    };
  }, [open, refresh]);

  return (
    <div ref={containerRef} className="relative" data-no-drag>
      <button
        ref={triggerRef}
        type="button"
        aria-label="Claude Code usage and limits"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className={`flex items-center gap-2 rounded-md px-2 py-1 text-xs hover:bg-bg-hover cursor-pointer focus-visible:outline focus-visible:outline-accent ${TONE[tone]}`}
      >
        <span>Claude</span>
        {primaryWindows.length ? (
          primaryWindows.map((window) => (
            <span key={window.id} className="tabular-nums">
              {window.id === "five_hour" ? "5h" : "7d"} {Math.round(window.usedPercent)}%
            </span>
          ))
        ) : (
          <span className="text-text-muted">{loading ? "…" : "—"}</span>
        )}
        {tone !== "normal" && (
          <span role="status" className="rounded bg-current/10 px-1.5">
            Limit ≥80%
          </span>
        )}
      </button>
      {open && (
        <div
          role="dialog"
          aria-label="Claude Code usage"
          className="absolute right-0 top-full mt-2 z-50 w-80 max-w-[calc(100vw-2rem)] rounded-xl border border-border bg-bg-secondary shadow-2xl p-4"
        >
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-sm font-medium text-text-primary">Claude Code usage</h2>
            <button
              ref={closeRef}
              type="button"
              aria-label="Close Claude Code usage"
              onClick={() => {
                setOpen(false);
                triggerRef.current?.focus();
              }}
              className="text-text-muted hover:text-text-primary cursor-pointer focus-visible:outline focus-visible:outline-accent"
            >
              <CloseIcon size={14} />
            </button>
          </div>
          {error || loading || windows.length ? (
            <PlanUsageContent usage={snapshot?.usage ?? null} error={error} />
          ) : (
            <p className="text-sm text-text-muted">
              No current usage data. Use Claude Code to update it.
            </p>
          )}
          {snapshot?.updatedAt && (
            <p className="mt-4 text-xs text-text-muted select-text">
              Last updated {new Date(snapshot.updatedAt).toLocaleString()}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
