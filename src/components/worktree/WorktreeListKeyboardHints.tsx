import { Kbd } from "../ui/Kbd";
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
          <Kbd>↑↓</Kbd> navigate
        </span>
        <span>
          <Kbd>{label("list.jump.0")}</Kbd> jump to first
        </span>
        <span>
          <Kbd>↵</Kbd> open
        </span>
        <span>
          <Kbd>{label("list.linear")}</Kbd> linear
        </span>
        {showNotes && (
          <span>
            <Kbd>{label("list.note")}</Kbd> notes
          </span>
        )}
        <span>
          <Kbd>{label("list.branch")}</Kbd> branch
        </span>
        <span>
          <Kbd>{label("list.path")}</Kbd> path
        </span>
        <span>
          <Kbd>{label("list.delete")}</Kbd> delete
        </span>
        <span>
          <Kbd>{label("app.search")}</Kbd> palette
        </span>
        <span>
          <Kbd>
            {label("app.history.back")} {label("app.history.forward")}
          </Kbd>{" "}
          history
        </span>
        <span>
          <Kbd>{label("list.refresh")}</Kbd> refresh
        </span>
      </div>
    </div>
  );
}
