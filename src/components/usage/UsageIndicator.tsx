import { useEffect, useRef, useState } from "react";
import { useEditorOcclusion } from "../../hooks/useEditorOcclusion";
import { PlanUsage, UsageWindow, usageTone, UsageTone } from "../../services/usage";
import { CloseIcon } from "../ui/Icons";
import { PlanUsageContent } from "./PlanUsageContent";

const TONE: Record<UsageTone, string> = {
  normal: "text-text-secondary",
  warning: "text-warning",
  danger: "text-danger",
};

interface UsageIndicatorProps {
  agentLabel: string;
  label: string;
  usage: PlanUsage | null;
  primaryWindows: UsageWindow[];
  loading?: boolean;
  error?: string | null;
  updatedAt?: number | null;
  emptyMessage?: string;
  planUrl?: string;
  onRefresh: () => Promise<void>;
}

function windowLabel(window: UsageWindow): string {
  if (window.label === "Weekly limit") return "7d";
  return window.label
    .replace(/-hour limit$/, "h")
    .replace(/-day limit$/, "d")
    .replace(/-minute limit$/, "m");
}

export function UsageIndicator({
  agentLabel,
  label,
  usage,
  primaryWindows,
  loading = false,
  error,
  updatedAt,
  emptyMessage,
  planUrl,
  onRefresh,
}: UsageIndicatorProps) {
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const windows = usage?.windows ?? [];
  const tone = usageTone(Math.max(0, ...windows.map((window) => window.usedPercent)));
  useEditorOcclusion(open);

  useEffect(() => {
    if (!open) return;
    let disposed = false;
    setRefreshError(null);
    void onRefresh().catch((e: unknown) => {
      if (!disposed) setRefreshError(String(e));
    });
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
        const controls = Array.from(
          dialogRef.current?.querySelectorAll<HTMLElement>("button, a[href]") ?? []
        );
        const current = controls.findIndex((control) => control === document.activeElement);
        const next = (current + (event.shiftKey ? -1 : 1) + controls.length) % controls.length;
        controls[next]?.focus();
      }
    };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", keyboard);
    return () => {
      disposed = true;
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", keyboard);
    };
  }, [open, onRefresh]);

  return (
    <div ref={containerRef} className="relative" data-no-drag>
      <button
        ref={triggerRef}
        type="button"
        aria-label={`${agentLabel} usage and limits`}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className={`flex items-center gap-2 rounded-md px-2 py-1 text-xs hover:bg-bg-hover cursor-pointer focus-visible:outline focus-visible:outline-accent ${TONE[tone]}`}
      >
        <span>{label}</span>
        {primaryWindows.length ? (
          primaryWindows.map((window) => (
            <span key={window.id} className="tabular-nums">
              {windowLabel(window)} {Math.round(window.usedPercent)}%
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
          ref={dialogRef}
          role="dialog"
          aria-label={`${agentLabel} usage`}
          className="absolute right-0 top-full mt-2 z-50 w-80 max-w-[calc(100vw-2rem)] rounded-xl border border-border bg-bg-secondary shadow-2xl p-4"
        >
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-sm font-medium text-text-primary">{agentLabel} usage</h2>
            <button
              ref={closeRef}
              type="button"
              aria-label={`Close ${agentLabel} usage`}
              onClick={() => {
                setOpen(false);
                triggerRef.current?.focus();
              }}
              className="text-text-muted hover:text-text-primary cursor-pointer focus-visible:outline focus-visible:outline-accent"
            >
              <CloseIcon size={14} />
            </button>
          </div>
          {error || refreshError || loading || windows.length || !emptyMessage ? (
            <PlanUsageContent usage={usage} error={error ?? refreshError} planUrl={planUrl} />
          ) : (
            <p className="text-sm text-text-muted">{emptyMessage}</p>
          )}
          {updatedAt && (
            <p className="mt-4 text-xs text-text-muted select-text">
              Last updated {new Date(updatedAt).toLocaleString()}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
