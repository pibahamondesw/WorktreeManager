import { useEffect, useRef } from "react";
import { AutomationSummary } from "../../services/automations";
import { EditorApp } from "../../types";
import { Modal } from "../ui/Modal";
import { AutomationList } from "./AutomationList";
import { AutomationDetail } from "./AutomationDetail";

interface AutomationsModalProps {
  open: boolean;
  onClose: () => void;
  automations: AutomationSummary[];
  error: string | null;
  /** Falls back to the first automation when unset or no longer listed. */
  selectedName: string | null;
  onSelect: (name: string) => void;
  editorApp: EditorApp;
}

export function AutomationsModal({
  open,
  onClose,
  automations,
  error,
  selectedName,
  onSelect,
  editorApp,
}: AutomationsModalProps) {
  const selected =
    automations.find((summary) => summary.entry.name === selectedName) ?? automations[0];
  const contentRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement;
    contentRef.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus();
    return () => {
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, [open]);

  return (
    <Modal open={open} onClose={onClose} title="Automations" size="settings" trapFocus>
      {open && (
        <div ref={contentRef} className="flex h-full min-h-0">
          <div className="w-52 shrink-0 border-r border-border p-3 overflow-y-auto bg-bg-tertiary/30">
            {error && (
              <p className="px-1 pb-2 text-xs text-danger break-words select-text">{error}</p>
            )}
            {automations.length === 0 && !error && (
              <p className="px-1 text-xs text-text-muted">No automations yet</p>
            )}
            <AutomationList
              automations={automations}
              selectedName={selected?.entry.name ?? null}
              onSelect={onSelect}
            />
          </div>
          <div
            id="automation-panel"
            role="tabpanel"
            aria-label={selected?.entry.name}
            className="flex-1 min-w-0 min-h-0"
          >
            {selected && (
              <AutomationDetail
                key={selected.entry.name}
                summary={selected}
                editorApp={editorApp}
              />
            )}
          </div>
        </div>
      )}
    </Modal>
  );
}
