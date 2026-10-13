import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../ui/Button";
import { CloseIcon } from "../ui/Icons";
import { byRepository, ChangeSet, ChatChanges, FileChange, totals } from "../../services/chat";
import { AgentId } from "../../types";
import { DiffLayout, FileDiffView } from "./FileDiffView";

export type ChangesScope = "conversation" | number;

const STATUS: Record<FileChange["status"], { letter: string; className: string }> = {
  added: { letter: "A", className: "text-success" },
  modified: { letter: "M", className: "text-warning" },
  deleted: { letter: "D", className: "text-danger" },
  renamed: { letter: "R", className: "text-accent-hover" },
};

const fileKey = (file: FileChange) => `${file.folder}\n${file.path}`;

export function LineCounts({ additions, deletions }: { additions: number; deletions: number }) {
  return (
    <span className="font-mono tabular-nums">
      <span className="text-success">+{additions}</span>{" "}
      <span className="text-danger">−{deletions}</span>
    </span>
  );
}

export function ChangeSummaryCard({ changes, onOpen }: { changes: ChangeSet; onOpen: () => void }) {
  const repositories = byRepository(changes.files);
  const count = changes.files.length;
  return (
    <button
      type="button"
      onClick={onOpen}
      className="w-full rounded-lg border border-border bg-bg-secondary/40 px-3 py-1.5 text-left text-xs hover:bg-bg-hover cursor-pointer flex flex-col gap-0.5"
    >
      {repositories.map((repository) => (
        <span key={repository.folder} className="flex items-center gap-2">
          <span className="text-text-muted">±</span>
          <span className="flex-1 min-w-0 truncate text-text-secondary">
            {repositories.length > 1 && (
              <span className="font-medium text-text-primary">{repository.name} · </span>
            )}
            {repository.files.length} {repository.files.length === 1 ? "file" : "files"} changed
          </span>
          <LineCounts {...totals(repository.files)} />
        </span>
      ))}
      <span className="sr-only">
        Review {count === 1 ? "1 changed file" : `${count} changed files`}
      </span>
    </button>
  );
}

function FileRow({
  file,
  selected,
  onSelect,
}: {
  file: FileChange;
  selected: boolean;
  onSelect: () => void;
}) {
  const slash = file.path.lastIndexOf("/");
  const status = STATUS[file.status];
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={selected || undefined}
      title={file.previousPath ? `${file.previousPath} → ${file.path}` : file.path}
      className={`w-full flex items-center gap-2 px-3 py-1 text-left text-xs cursor-pointer ${
        selected ? "bg-bg-active text-text-primary" : "text-text-secondary hover:bg-bg-hover"
      }`}
    >
      <span title={file.status} className={`w-3 shrink-0 font-mono ${status.className}`}>
        {status.letter}
      </span>
      <span className="flex-1 min-w-0 truncate">
        <span className="text-text-primary">{file.path.slice(slash + 1)}</span>
        {slash > 0 && <span className="text-text-muted"> {file.path.slice(0, slash)}</span>}
      </span>
      {file.binary ? (
        <span className="text-text-muted">bin</span>
      ) : (
        <LineCounts additions={file.additions} deletions={file.deletions} />
      )}
    </button>
  );
}

function scopeLabel(index: number, prompt: string | undefined) {
  const text = prompt?.replace(/\s+/g, " ").trim() ?? "";
  return `Turn ${index + 1}${text ? ` · ${text.length > 48 ? `${text.slice(0, 48)}…` : text}` : ""}`;
}

export function ChangesPanel({
  taskId,
  agent,
  changes,
  prompts,
  scope: initialScope,
  onClose,
}: {
  taskId: string;
  agent: AgentId;
  changes: ChatChanges;
  prompts: Record<string, string>;
  scope: ChangesScope;
  onClose: () => void;
}) {
  const [scope, setScope] = useState<ChangesScope>(initialScope);
  const [layout, setLayout] = useState<DiffLayout>("unified");
  const [hideFormatting, setHideFormatting] = useState(false);
  const changeSet = scope === "conversation" ? changes.conversation : changes.turns[scope]?.changes;
  const files = useMemo(() => changeSet?.files ?? [], [changeSet]);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const selected = files.find((file) => fileKey(file) === selectedKey) ?? files[0];
  const repositories = byRepository(files);
  const panelRef = useRef<HTMLElement>(null);

  useEffect(() => panelRef.current?.focus(), []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <section
      ref={panelRef}
      tabIndex={-1}
      aria-label="Changes"
      className="@container absolute inset-0 z-20 flex flex-col bg-bg-primary outline-none"
    >
      <header className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2 text-xs">
        <h2 className="text-sm font-semibold text-text-primary">Changes</h2>
        <select
          aria-label="Changes scope"
          value={String(scope)}
          onChange={(e) =>
            setScope(e.target.value === "conversation" ? "conversation" : Number(e.target.value))
          }
          className="min-w-0 max-w-full bg-bg-tertiary border border-border rounded-md px-2 py-1 text-xs text-text-primary cursor-pointer"
        >
          <option value="conversation">Whole conversation</option>
          {changes.turns.map((turn, index) => (
            <option key={turn.changes.before + turn.changes.after + index} value={index}>
              {scopeLabel(index, prompts[turn.items[0]])}
            </option>
          ))}
        </select>
        <span className="text-text-muted">
          {files.length} {files.length === 1 ? "file" : "files"}
        </span>
        <LineCounts {...totals(files)} />
        <span className="flex-1" />
        <label className="flex items-center gap-1.5 text-text-secondary cursor-pointer">
          <input
            type="checkbox"
            checked={hideFormatting}
            onChange={(e) => setHideFormatting(e.target.checked)}
          />
          Hide formatting
        </label>
        <div role="group" aria-label="Diff layout" className="flex rounded-md border border-border">
          {(["unified", "split"] as const).map((option) => (
            <button
              key={option}
              type="button"
              aria-pressed={layout === option}
              onClick={() => setLayout(option)}
              className={`px-2 py-0.5 capitalize cursor-pointer ${
                layout === option ? "bg-bg-active text-text-primary" : "text-text-secondary"
              }`}
            >
              {option}
            </button>
          ))}
        </div>
        <Button variant="ghost" onClick={onClose} aria-label="Close changes" className="h-7 px-1.5">
          <CloseIcon size={14} />
        </Button>
      </header>
      {files.length === 0 ? (
        <p className="p-6 text-sm text-text-muted text-center">No file changes.</p>
      ) : (
        <div className="flex-1 min-h-0 flex flex-col @2xl:flex-row">
          <nav
            aria-label="Changed files"
            className="shrink-0 max-h-48 @2xl:max-h-none @2xl:w-72 overflow-y-auto border-b @2xl:border-b-0 @2xl:border-r border-border py-1"
          >
            {repositories.map((repository) => (
              <div key={repository.folder}>
                {repositories.length > 1 && (
                  <div className="flex items-center gap-2 px-3 pt-2 pb-1 text-[11px] font-medium text-text-muted">
                    <span className="flex-1 truncate select-text" title={repository.folder}>
                      {repository.name}
                    </span>
                    <LineCounts {...totals(repository.files)} />
                  </div>
                )}
                {repository.files.map((file) => (
                  <FileRow
                    key={fileKey(file)}
                    file={file}
                    selected={file === selected}
                    onSelect={() => setSelectedKey(fileKey(file))}
                  />
                ))}
              </div>
            ))}
          </nav>
          {selected && changeSet && (
            <div className="flex-1 min-w-0 min-h-0 flex flex-col">
              <div className="shrink-0 px-3 py-1.5 border-b border-border text-xs font-mono text-text-secondary select-text truncate">
                {selected.previousPath && `${selected.previousPath} → `}
                {selected.path}
              </div>
              <div className="flex-1 min-h-0 overflow-auto">
                <FileDiffView
                  taskId={taskId}
                  agent={agent}
                  changes={changeSet}
                  file={selected}
                  layout={layout}
                  hideFormatting={hideFormatting}
                />
              </div>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
