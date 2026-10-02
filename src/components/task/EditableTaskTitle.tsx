import { CSSProperties, KeyboardEvent, useState } from "react";
import { PencilIcon, SpinnerIcon } from "../ui/Icons";

interface EditableTaskTitleProps {
  title: string;
  textClassName: string;
  titleStyle?: CSSProperties;
  onRename?: (title: string) => Promise<unknown>;
  onError?: (message: string) => void;
}

const stop = (event: { stopPropagation: () => void }) => event.stopPropagation();

export function EditableTaskTitle({
  title,
  textClassName,
  titleStyle,
  onRename,
  onError,
}: EditableTaskTitleProps) {
  const [draft, setDraft] = useState<string | null>(null);
  const [savingTitle, setSavingTitle] = useState<string | null>(null);

  const save = async (value: string) => {
    setDraft(null);
    const next = value.trim();
    if (!onRename || !next || next === title) return;
    setSavingTitle(next);
    try {
      await onRename(next);
    } catch (error) {
      onError?.(error instanceof Error ? error.message : String(error));
    } finally {
      setSavingTitle(null);
    }
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    event.stopPropagation();
    if (event.key === "Enter" && !event.nativeEvent.isComposing)
      void save(event.currentTarget.value);
    if (event.key === "Escape") setDraft(null);
  };

  if (draft !== null) {
    return (
      <div className="flex items-center min-w-0 flex-1">
        <input
          autoFocus
          aria-label="Task title"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={handleKeyDown}
          onBlur={() => setDraft(null)}
          onClick={stop}
          onFocus={(event) => event.currentTarget.select()}
          className={`${textClassName} flex-1 min-w-0 -mx-1.5 -my-px px-1.5 rounded-md border border-accent bg-bg-primary outline-none shadow-[0_0_0_3px_color-mix(in_srgb,var(--color-accent)_20%,transparent)] select-text`}
        />
      </div>
    );
  }

  const shownTitle = savingTitle ?? title;
  return (
    <div className="group/title flex items-center gap-1 min-w-0">
      <h3
        data-task-part="title"
        className={`${textClassName} truncate min-w-0 ${savingTitle ? "opacity-60" : ""}`}
        style={titleStyle}
        title={shownTitle}
      >
        {shownTitle}
      </h3>
      {savingTitle ? (
        <span className="w-5 h-5 flex items-center justify-center flex-shrink-0 text-text-muted">
          <SpinnerIcon size={12} />
        </span>
      ) : (
        onRename && (
          <button
            type="button"
            aria-label="Rename task"
            title="Rename"
            onClick={(event) => {
              event.stopPropagation();
              setDraft(title);
            }}
            className="w-5 h-5 flex items-center justify-center flex-shrink-0 rounded-md text-text-muted hover:text-text-primary hover:bg-bg-hover cursor-pointer opacity-0 transition-opacity group-hover/title:opacity-100 group-hover/title:delay-300 focus-visible:opacity-100 focus-visible:outline focus-visible:outline-accent"
          >
            <PencilIcon size={12} />
          </button>
        )
      )}
    </div>
  );
}
