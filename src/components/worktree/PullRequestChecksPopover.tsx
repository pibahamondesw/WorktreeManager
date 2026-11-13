import { FocusEvent, KeyboardEvent, ReactNode, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { openUrl } from "@tauri-apps/plugin-opener";
import { fetchGithubPrChecks } from "../../services/github";
import { GithubPrCheck, PullRequestInfo } from "../../types";
import { CheckIcon, CloseIcon, SpinnerIcon } from "../ui/Icons";

type ChecksLoad =
  | { status: "loading" }
  | { status: "loaded"; checks: GithubPrCheck[] }
  | { status: "error"; message: string };

const POPOVER_WIDTH_REM = 20;
const VIEWPORT_MARGIN_PX = 8;
const ANCHOR_GAP_PX = 6;
const MIN_HEIGHT_BELOW_PX = 160;

function CheckStateIcon({ state }: { state: GithubPrCheck["state"] }) {
  if (state === "passing") return <CheckIcon size={12} className="text-success flex-shrink-0" />;
  if (state === "failing") return <CloseIcon size={12} className="text-danger flex-shrink-0" />;
  if (state === "running") return <SpinnerIcon size={12} className="text-warning flex-shrink-0" />;
  return <span className="w-3 flex-shrink-0 text-center text-text-muted">–</span>;
}

function popoverPosition(anchor: HTMLElement) {
  const rect = anchor.getBoundingClientRect();
  const fontSize = Number.parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
  const width = Math.min(POPOVER_WIDTH_REM * fontSize, window.innerWidth - 2 * VIEWPORT_MARGIN_PX);
  const left = Math.max(
    VIEWPORT_MARGIN_PX,
    Math.min(rect.left, window.innerWidth - width - VIEWPORT_MARGIN_PX)
  );
  const spaceBelow = window.innerHeight - rect.bottom - ANCHOR_GAP_PX - VIEWPORT_MARGIN_PX;
  const spaceAbove = rect.top - ANCHOR_GAP_PX - VIEWPORT_MARGIN_PX;
  if (spaceBelow < MIN_HEIGHT_BELOW_PX && spaceAbove > spaceBelow) {
    return {
      width,
      left,
      bottom: window.innerHeight - rect.top + ANCHOR_GAP_PX,
      maxHeight: spaceAbove,
    };
  }
  return { width, left, top: rect.bottom + ANCHOR_GAP_PX, maxHeight: spaceBelow };
}

interface PullRequestChecksPopoverProps {
  pr: PullRequestInfo;
  children: ReactNode;
  onOpenError?: (msg: string) => void;
}

export function PullRequestChecksPopover({
  pr,
  children,
  onOpenError,
}: PullRequestChecksPopoverProps) {
  const [position, setPosition] = useState<ReturnType<typeof popoverPosition> | null>(null);
  const [load, setLoad] = useState<ChecksLoad>({ status: "loading" });
  const buttonRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  const requestRef = useRef(0);
  const popoverId = useId();
  const open = !!position;

  const close = (restoreFocus = false) => {
    setPosition(null);
    if (restoreFocus) buttonRef.current?.focus();
  };

  useEffect(() => {
    if (!open) return;
    popoverRef.current?.focus();
    const dismiss = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!popoverRef.current?.contains(target) && !buttonRef.current?.contains(target))
        setPosition(null);
    };
    const dismissOnScroll = (event: Event) => {
      if (!(event.target instanceof Node) || !popoverRef.current?.contains(event.target))
        setPosition(null);
    };
    const dismissOnResize = () => setPosition(null);
    document.addEventListener("pointerdown", dismiss);
    window.addEventListener("resize", dismissOnResize);
    window.addEventListener("scroll", dismissOnScroll, true);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      window.removeEventListener("resize", dismissOnResize);
      window.removeEventListener("scroll", dismissOnScroll, true);
    };
  }, [open]);

  const toggle = () => {
    if (open) return close();
    if (!buttonRef.current) return;
    setPosition(popoverPosition(buttonRef.current));
    setLoad({ status: "loading" });
    const request = ++requestRef.current;
    fetchGithubPrChecks(pr)
      .then((checks) => {
        if (request === requestRef.current) setLoad({ status: "loaded", checks });
      })
      .catch((error) => {
        if (request === requestRef.current) setLoad({ status: "error", message: String(error) });
      });
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      close(true);
    }
  };

  const closeWhenFocusLeaves = (event: FocusEvent<HTMLDivElement>) => {
    const next = event.relatedTarget;
    if (!(next instanceof Node)) return;
    if (popoverRef.current?.contains(next) || buttonRef.current?.contains(next)) return;
    setPosition(null);
  };

  const openCheck = (url: string) => {
    openUrl(url).catch(() => onOpenError?.("Could not open the check"));
  };

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? popoverId : undefined}
        title="Show checks"
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") event.stopPropagation();
          if (event.key === "Escape" && open) {
            event.preventDefault();
            event.stopPropagation();
            close();
          }
        }}
        onClick={(event) => {
          event.stopPropagation();
          toggle();
        }}
        className="pointer-events-auto cursor-pointer rounded-md hover:opacity-80 focus-visible:outline-2 focus-visible:outline-accent"
      >
        {children}
      </button>
      {position &&
        createPortal(
          <div
            ref={popoverRef}
            id={popoverId}
            role="dialog"
            aria-label={`Checks for #${pr.number}`}
            tabIndex={-1}
            onClick={(event) => event.stopPropagation()}
            onMouseDown={(event) => event.stopPropagation()}
            onKeyDown={handleKeyDown}
            onBlur={closeWhenFocusLeaves}
            style={position}
            className="fixed z-50 overflow-y-auto rounded-lg focus:outline-none border border-border bg-bg-secondary p-1 text-xs shadow-xl"
          >
            {load.status === "loading" && (
              <div className="flex items-center gap-2 px-2 py-1.5 text-text-muted">
                <SpinnerIcon size={12} />
                Loading checks…
              </div>
            )}
            {load.status === "error" && (
              <div className="px-2 py-1.5 text-danger select-text">{load.message}</div>
            )}
            {load.status === "loaded" && load.checks.length === 0 && (
              <div className="px-2 py-1.5 text-text-muted">No checks reported</div>
            )}
            {load.status === "loaded" &&
              load.checks.map((check, index) => {
                const content = (
                  <>
                    <CheckStateIcon state={check.state} />
                    <span className="truncate">{check.name}</span>
                  </>
                );
                const rowClass =
                  "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-text-primary";
                return check.url ? (
                  <button
                    key={`${check.name}-${index}`}
                    type="button"
                    title={`${check.name}: ${check.state}`}
                    onClick={() => openCheck(check.url!)}
                    className={`${rowClass} cursor-pointer hover:bg-bg-hover focus:bg-bg-hover focus:outline-none`}
                  >
                    {content}
                  </button>
                ) : (
                  <div
                    key={`${check.name}-${index}`}
                    title={`${check.name}: ${check.state}`}
                    className={rowClass}
                  >
                    {content}
                  </div>
                );
              })}
          </div>,
          document.body
        )}
    </>
  );
}
