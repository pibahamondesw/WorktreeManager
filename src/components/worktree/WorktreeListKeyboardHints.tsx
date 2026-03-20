export function WorktreeListKeyboardHints() {
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
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">⌘B</kbd> branch
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">⌘⇧C</kbd> path
        </span>
        <span>
          <kbd className="px-1 py-0.5 bg-bg-tertiary rounded">⌘D</kbd> delete
        </span>
      </div>
    </div>
  );
}
