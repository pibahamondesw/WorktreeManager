interface WorktreeListKeyboardHintsProps {
  /** Only advertise the notes shortcut when the workspace has a notes folder. */
  showNotes?: boolean;
}

export function WorktreeListKeyboardHints({ showNotes }: WorktreeListKeyboardHintsProps) {
  return (
    <div className="flex-shrink-0 px-6 py-2 border-t border-border">
      <div className="flex items-center gap-3 text-[10px] text-text-muted font-mono flex-wrap">
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">↑↓</kbd> navigate
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">1-9</kbd> jump
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">↵</kbd> open
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">L</kbd> linear
        </span>
        {showNotes && (
          <span>
            <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">O</kbd> notes
          </span>
        )}
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">⌘B</kbd> branch
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">⌘⇧C</kbd> path
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">⌘D</kbd> delete
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">⌘K</kbd> search
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">⌘R</kbd> refresh
        </span>
      </div>
    </div>
  );
}
