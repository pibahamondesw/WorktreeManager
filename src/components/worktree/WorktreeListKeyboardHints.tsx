import { useShortcutLabels } from "../../shortcuts/runtime";
interface WorktreeListKeyboardHintsProps {
  /** Only advertise the notes shortcut when the workspace has a notes folder. */
  showNotes?: boolean;
}

export function WorktreeListKeyboardHints({ showNotes }: WorktreeListKeyboardHintsProps) {
  const label = useShortcutLabels();
  return (
    <div className="flex-shrink-0 px-6 py-2 border-t border-border">
      <div className="flex items-center gap-3 text-[0.625rem] text-text-muted font-mono flex-wrap">
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">↑↓</kbd> navigate
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">{label("list.jump.0")}</kbd> jump to
          first
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">↵</kbd> open
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">{label("list.linear")}</kbd> linear
        </span>
        {showNotes && (
          <span>
            <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">{label("list.note")}</kbd> notes
          </span>
        )}
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">{label("list.branch")}</kbd> branch
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">{label("list.path")}</kbd> path
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">{label("list.delete")}</kbd> delete
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">{label("app.search")}</kbd> palette
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">
            {label("app.history.back")} {label("app.history.forward")}
          </kbd>{" "}
          history
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">{label("list.refresh")}</kbd> refresh
        </span>
      </div>
    </div>
  );
}
