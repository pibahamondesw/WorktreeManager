import { KeyboardEventHandler, ReactNode, useEffect, useRef } from "react";
import { CloseIcon } from "./Icons";
import { useEditorOcclusion } from "../../hooks/useEditorOcclusion";

interface ModalProps {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  wide?: boolean;
  size?: "settings";
  onKeyDown?: KeyboardEventHandler<HTMLDivElement>;
}

export function Modal({ open, onClose, title, children, wide, size, onKeyDown }: ModalProps) {
  useEditorOcclusion(open);
  const overlayRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleEsc = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || e.isComposing) return;
      const dialogs = Array.from(document.querySelectorAll('[role="dialog"]'));
      if (dialogs.at(-1) !== overlayRef.current?.querySelector('[role="dialog"]')) return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    if (open) document.addEventListener("keydown", handleEsc);
    return () => document.removeEventListener("keydown", handleEsc);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      ref={overlayRef}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm motion-fade"
      onClick={(e) => {
        if (e.target === overlayRef.current) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onKeyDown={onKeyDown}
        className={`motion-rise bg-bg-secondary border border-border rounded-xl shadow-2xl flex flex-col max-h-[80vh] max-w-[92vw] ${
          size === "settings" ? "w-[56rem] h-[40rem]" : wide ? "w-[40rem]" : "w-[30rem]"
        }`}
      >
        <div className="flex items-center justify-between px-6 py-4 border-b border-border">
          <h2 className="text-base font-semibold text-text-primary">{title}</h2>
          <button
            onClick={onClose}
            aria-label={`Close ${title}`}
            className="text-text-muted hover:text-text-primary transition-colors cursor-pointer"
          >
            <CloseIcon />
          </button>
        </div>
        <div className="flex-1 min-h-0 overflow-y-auto">{children}</div>
      </div>
    </div>
  );
}
