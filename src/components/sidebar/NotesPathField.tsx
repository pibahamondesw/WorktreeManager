import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "../ui/Button";
import { CloseIcon } from "../ui/Icons";

interface NotesPathFieldProps {
  value: string;
  onChange: (value: string) => void;
}

/** Optional Obsidian `task-logs/` folder. Empty disables task notes for the workspace. */
export function NotesPathField({ value, onChange }: NotesPathFieldProps) {
  const browse = async () => {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: "Select task-logs folder",
    });
    if (selected) onChange(selected as string);
  };

  return (
    <div className="flex flex-col gap-2">
      <label className="text-sm font-medium text-text-secondary">Notes folder (optional)</label>
      <p className="text-xs text-text-muted">
        An Obsidian <span className="font-mono">task-logs/</span> folder. Each task gets a note,
        archived when the task is deleted. Leave empty to disable. See vault-kit/README.md.
      </p>
      {value ? (
        <div className="flex items-center gap-2 rounded-lg border border-border bg-bg-tertiary px-3 py-2">
          <p className="flex-1 text-xs text-text-muted font-mono truncate" title={value}>
            {value}
          </p>
          <button
            onClick={() => onChange("")}
            className="w-6 h-6 flex items-center justify-center rounded text-text-muted hover:text-danger transition-colors cursor-pointer flex-shrink-0"
            title="Disable task notes"
          >
            <CloseIcon size={12} />
          </button>
        </div>
      ) : (
        <Button variant="secondary" onClick={browse} className="self-start">
          Choose folder
        </Button>
      )}
    </div>
  );
}
