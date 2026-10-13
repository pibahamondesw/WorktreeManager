import { KeyboardEvent, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { HistoryAction } from "../../services/chat";
import { RewindIcon } from "../ui/Icons";

interface HistoryMenuProps {
  disabled?: boolean;
  canRewind: boolean;
  onPick: (action: HistoryAction) => void;
}

const ACTIONS = [
  { id: "conversation", label: "Fork conversation from here" },
  { id: "code", label: "Rewind code to here" },
  { id: "both", label: "Fork conversation and rewind code" },
] as const;

export function HistoryMenu({ disabled, canRewind, onPick }: HistoryMenuProps) {
  const [position, setPosition] = useState<{ top: number; left: number; width: number } | null>(
    null
  );
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const open = !!position && !disabled;

  useEffect(() => {
    if (disabled) setPosition(null);
  }, [disabled]);

  useLayoutEffect(() => {
    if (!open) return;
    const bottom = menuRef.current?.getBoundingClientRect().bottom ?? 0;
    if (bottom > window.innerHeight - 8) {
      setPosition(
        (current) =>
          current && { ...current, top: Math.max(8, current.top - bottom + window.innerHeight - 8) }
      );
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    menuRef.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    const dismiss = (event: PointerEvent) => {
      if (
        !menuRef.current?.contains(event.target as Node) &&
        !buttonRef.current?.contains(event.target as Node)
      )
        setPosition(null);
    };
    const close = () => setPosition(null);
    const closeOnScroll = (event: Event) => {
      if (!(event.target instanceof Node) || !menuRef.current?.contains(event.target)) close();
    };
    document.addEventListener("pointerdown", dismiss);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", closeOnScroll, true);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", closeOnScroll, true);
    };
  }, [open]);

  const toggle = () => {
    if (open) {
      setPosition(null);
      return;
    }
    const rect = buttonRef.current?.getBoundingClientRect();
    if (!rect) return;
    const fontSize = Number.parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
    const width = Math.min(17.5 * fontSize, window.innerWidth - 16);
    const height = 10 * fontSize;
    setPosition({
      width,
      left: Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8)),
      top:
        rect.bottom + height + 6 <= window.innerHeight
          ? rect.bottom + 6
          : Math.max(8, rect.top - height - 6),
    });
  };

  const navigate = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape" || event.key === "Tab") {
      event.stopPropagation();
      setPosition(null);
      buttonRef.current?.focus();
      if (event.key === "Escape") event.preventDefault();
      return;
    }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    event.stopPropagation();
    const buttons = Array.from(
      menuRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []
    );
    const active = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const index =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? buttons.length - 1
          : (active + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length;
    buttons[index]?.focus();
  };

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        aria-label="Rewind or fork"
        title="Rewind or fork"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        disabled={disabled}
        onClick={toggle}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            event.stopPropagation();
            toggle();
          }
        }}
        className={`absolute -top-2 -right-2 flex size-7 items-center justify-center rounded-full border border-border bg-bg-primary text-text-muted hover:text-text-primary hover:bg-bg-hover focus-visible:outline-2 focus-visible:outline-accent transition-opacity cursor-pointer disabled:cursor-default disabled:text-text-muted ${open ? "opacity-100" : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100"}`}
      >
        <RewindIcon />
      </button>
      {open &&
        position &&
        createPortal(
          <div
            ref={menuRef}
            id={menuId}
            role="menu"
            tabIndex={-1}
            aria-label="Message history"
            onKeyDown={navigate}
            style={position}
            className="fixed z-50 max-h-[calc(100vh-1rem)] overflow-y-auto rounded-lg border border-border bg-bg-secondary p-1 shadow-xl"
          >
            {ACTIONS.map((action) => (
              <button
                key={action.id}
                role="menuitem"
                type="button"
                disabled={action.id !== "conversation" && !canRewind}
                title={
                  action.id !== "conversation" && !canRewind
                    ? "No code checkpoint is available for this message"
                    : undefined
                }
                onClick={() => {
                  setPosition(null);
                  buttonRef.current?.focus();
                  onPick(action.id);
                }}
                className="w-full rounded-md px-3 py-2.5 text-left text-sm text-text-primary hover:bg-bg-hover focus:bg-bg-hover focus:outline-none cursor-pointer disabled:text-text-muted disabled:opacity-40 disabled:cursor-default"
              >
                {action.label}
              </button>
            ))}
          </div>,
          document.body
        )}
    </>
  );
}
