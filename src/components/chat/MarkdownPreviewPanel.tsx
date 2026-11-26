import { ReactNode, RefObject, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Button } from "../ui/Button";
import { ChevronLeftIcon, CloseIcon, DocumentIcon, RefreshIcon, SpinnerIcon } from "../ui/Icons";
import { MarkdownText } from "./ChatItemView";
import { FilePreview } from "./FilePreview";
import { usePreviewFile } from "./previewFile";
import { chatWriteFile } from "../../services/chat";
import {
  directoryOf,
  documentSummary,
  fileName,
  MarkdownSource,
  relativeTo,
  splitFrontmatter,
} from "./markdownDocument";
import {
  MarkdownPreviewContext,
  MarkdownPreviewRequest,
  useMarkdownDocument,
} from "./markdownPreviewContext";

const OUTLINE_MIN_HEADINGS = 3;

function withLineEndingsOf(base: string, text: string) {
  return base.includes("\r\n") ? text.replace(/\r?\n/g, "\r\n") : text;
}

export interface FileEdit {
  base: string;
  text: string;
}

export interface UnsavedFileEdit extends FileEdit {
  source: MarkdownSource;
}

const hasUnsavedText = (edit: FileEdit) => withLineEndingsOf(edit.base, edit.text) !== edit.base;

const ACTIVE_HEADING_OFFSET = 96;

interface OutlineEntry {
  element: HTMLElement;
  level: number;
  text: string;
}

function useOutline(articleRef: RefObject<HTMLElement | null>, text: string) {
  const [outline, setOutline] = useState<OutlineEntry[]>([]);
  useLayoutEffect(() => {
    const headings = articleRef.current?.querySelectorAll<HTMLElement>("h1, h2, h3") ?? [];
    setOutline(
      [...headings].map((element) => ({
        element,
        level: Number(element.tagName[1]),
        text: element.textContent ?? "",
      }))
    );
  }, [articleRef, text]);
  return outline;
}

function activeHeading(scroller: HTMLElement, outline: OutlineEntry[]) {
  const top = scroller.getBoundingClientRect().top + ACTIVE_HEADING_OFFSET;
  let active = 0;
  outline.forEach((entry, index) => {
    if (entry.element.getBoundingClientRect().top <= top) active = index;
  });
  return active;
}

export function MarkdownPreviewPanel({
  request,
  folders,
  footer,
  editable,
  error,
  busy = false,
  initialFileEdit = null,
  onUnsavedFileEdit,
  onClose,
}: {
  request: MarkdownPreviewRequest;
  folders: string[];
  footer?: ReactNode;
  error?: string | null;
  editable?: { text: string; onChange: (text: string) => void; disabled: boolean };
  busy?: boolean;
  initialFileEdit?: FileEdit | null;
  onUnsavedFileEdit?: (edit: UnsavedFileEdit | null) => void;
  onClose: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [history, setHistory] = useState<MarkdownSource[]>([request.source]);
  const [revision, setRevision] = useState(0);
  const [active, setActive] = useState(0);
  const source = history[history.length - 1];
  const loaded = useMarkdownDocument(source, folders, revision);
  const codeFile = usePreviewFile(
    source.kind === "code" ? source.path : null,
    folders,
    source.kind === "code" ? (source.base ?? null) : null,
    revision
  );
  const canEdit = editable && history.length === 1;
  const [fileEdit, setFileEdit] = useState<FileEdit | null>(initialFileEdit);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [confirmingClose, setConfirmingClose] = useState(false);
  const editableText =
    source.kind === "code" && codeFile.kind === "ready" && codeFile.content.kind === "text"
      ? codeFile.content.text
      : null;
  const unsaved = fileEdit !== null && hasUnsavedText(fileEdit);
  const changeFileEdit = (edit: FileEdit | null) => {
    setFileEdit(edit);
    onUnsavedFileEdit?.(edit && hasUnsavedText(edit) ? { ...edit, source } : null);
  };
  const requestClose = () => (unsaved ? setConfirmingClose(true) : onClose());
  const stopEditing = () => {
    changeFileEdit(null);
    setSaveError(null);
    setConfirmingClose(false);
  };
  const saveFile = async (edit: FileEdit, path: string) => {
    setSaving(true);
    setSaveError(null);
    try {
      await chatWriteFile(folders, path, edit.base, withLineEndingsOf(edit.base, edit.text));
      stopEditing();
      setRevision((current) => current + 1);
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  };
  const text = canEdit ? editable.text : loaded.kind === "ready" ? loaded.text : "";
  const path =
    source.kind === "code"
      ? codeFile.kind === "ready"
        ? codeFile.path
        : undefined
      : loaded.kind === "ready"
        ? loaded.document?.path
        : undefined;
  const { entries, body } = useMemo(() => splitFrontmatter(text), [text]);
  const summary = useMemo(() => documentSummary(text), [text]);
  const panelRef = useRef<HTMLElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const articleRef = useRef<HTMLElement>(null);
  const outline = useOutline(articleRef, editing ? "" : body);
  const showOutline = outline.length >= OUTLINE_MIN_HEADINGS;

  const title =
    summary.title ?? (source.kind === "inline" ? source.title : fileName(path ?? source.path));

  const nested = useMemo(
    () => ({
      folders,
      base: path ? directoryOf(path) : undefined,
      open: ({ source: next }: MarkdownPreviewRequest) => {
        const base = path ? [directoryOf(path), ...folders] : folders;
        const resolved =
          next.kind === "code"
            ? { ...next, base: path ? directoryOf(path) : undefined }
            : next.kind === "file" && !next.folders
              ? { ...next, folders: base }
              : next;
        setHistory((current) => [...current, resolved]);
      },
    }),
    [folders, path]
  );

  const requestCloseRef = useRef(requestClose);
  useEffect(() => {
    requestCloseRef.current = requestClose;
  });

  useEffect(() => {
    const returnFocus = document.activeElement;
    panelRef.current?.focus();
    return () => {
      if (returnFocus instanceof HTMLElement) returnFocus.focus();
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      requestCloseRef.current();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
    setActive(0);
  }, [source]);

  return (
    <section
      ref={panelRef}
      tabIndex={-1}
      aria-label={`Preview of ${title}`}
      className="markdown-preview @container absolute inset-0 z-20 flex flex-col bg-bg-primary outline-none"
    >
      <header className="flex items-center gap-2 border-b border-border px-3 py-2">
        {history.length > 1 && !fileEdit && (
          <Button
            variant="ghost"
            onClick={() => setHistory((current) => current.slice(0, -1))}
            aria-label="Back"
            title="Back"
            className="h-7 px-1.5"
          >
            <ChevronLeftIcon />
          </Button>
        )}
        <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-accent/12 text-accent-hover">
          <DocumentIcon size={15} />
        </span>
        <div className="flex-1 min-w-0">
          <h2 className="truncate text-sm font-semibold text-text-primary select-text">{title}</h2>
          {path && (
            <p title={path} className="truncate font-mono text-[11px] text-text-muted select-text">
              {relativeTo(path, folders)}
            </p>
          )}
        </div>
        {canEdit && (
          <Button
            variant="secondary"
            className="h-7 px-2 text-xs"
            disabled={editable.disabled}
            onClick={() => setEditing((current) => !current)}
          >
            {editing ? "Preview plan" : "Edit plan"}
          </Button>
        )}
        {editableText !== null && !fileEdit && (
          <Button
            variant="secondary"
            className="h-7 px-2 text-xs"
            onClick={() => changeFileEdit({ base: editableText, text: editableText })}
          >
            Edit file
          </Button>
        )}
        {fileEdit && (
          <Button variant="ghost" className="h-7 px-2 text-xs" onClick={stopEditing}>
            Cancel
          </Button>
        )}
        {fileEdit && codeFile.kind === "ready" && (
          <Button
            className="h-7 px-2 text-xs"
            disabled={busy || !unsaved}
            loading={saving}
            onClick={() => void saveFile(fileEdit, codeFile.path)}
          >
            Save
          </Button>
        )}
        {source.kind !== "inline" && !fileEdit && (
          <Button
            variant="ghost"
            onClick={() => setRevision((current) => current + 1)}
            aria-label="Reload document"
            title="Reload document"
            className="h-7 px-1.5"
          >
            <RefreshIcon />
          </Button>
        )}
        <Button
          variant="ghost"
          onClick={requestClose}
          aria-label="Close preview"
          title="Close preview (Esc)"
          className="h-7 px-1.5"
        >
          <CloseIcon size={14} />
        </Button>
      </header>
      <div className="flex-1 min-h-0 flex">
        {!editing && showOutline && loaded.kind === "ready" && (
          <nav
            aria-label="Outline"
            className="hidden @3xl:block w-60 shrink-0 overflow-y-auto border-r border-border py-4"
          >
            {outline.map((entry, index) => (
              <button
                key={index}
                type="button"
                aria-current={index === active || undefined}
                onClick={() => entry.element.scrollIntoView({ behavior: "smooth", block: "start" })}
                style={{ paddingLeft: `${0.75 + (entry.level - 1) * 0.75}rem` }}
                className={`block w-full truncate py-1 pr-3 text-left text-xs cursor-pointer border-l-2 ${
                  index === active
                    ? "border-accent text-text-primary"
                    : "border-transparent text-text-secondary hover:text-text-primary"
                }`}
              >
                {entry.text}
              </button>
            ))}
          </nav>
        )}
        <div
          ref={scrollRef}
          onScroll={(e) => showOutline && setActive(activeHeading(e.currentTarget, outline))}
          className="chat-scroll flex-1 min-w-0 overflow-y-auto"
        >
          {fileEdit ? (
            <textarea
              aria-label="File contents"
              value={fileEdit.text}
              onChange={(e) => changeFileEdit({ ...fileEdit, text: e.target.value })}
              spellCheck={false}
              className="diff-code h-full min-h-64 w-full resize-none bg-bg-primary p-4 text-text-primary outline-none"
            />
          ) : (
            source.kind === "code" && <FilePreview file={codeFile} line={source.line} />
          )}
          {loaded.kind === "loading" && (
            <div className="flex items-center justify-center gap-2 p-8 text-xs text-text-muted">
              <SpinnerIcon size={12} /> Loading…
            </div>
          )}
          {loaded.kind === "error" && (
            <p role="alert" className="p-8 text-center text-sm text-danger select-text">
              {loaded.message}
            </p>
          )}
          {canEdit && editing ? (
            <textarea
              aria-label="Plan markdown"
              value={editable.text}
              onChange={(e) => editable.onChange(e.target.value)}
              disabled={editable.disabled}
              className="h-full min-h-64 w-full resize-none bg-bg-primary p-6 font-mono text-sm text-text-primary outline-none"
            />
          ) : (
            source.kind !== "code" &&
            loaded.kind === "ready" && (
              <article ref={articleRef} className="mx-auto max-w-3xl px-6 py-8 @2xl:px-10">
                {entries.length > 0 && (
                  <dl className="markdown-frontmatter mb-6 grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 rounded-lg border border-border bg-bg-secondary/60 px-4 py-3 text-xs select-text">
                    {entries.map(([key, value]) => (
                      <div key={key} className="contents">
                        <dt className="text-text-muted">{key}</dt>
                        <dd className="min-w-0 break-words text-text-secondary">{value}</dd>
                      </div>
                    ))}
                  </dl>
                )}
                <MarkdownPreviewContext.Provider value={nested}>
                  <MarkdownText text={body} className="markdown-document" />
                </MarkdownPreviewContext.Provider>
              </article>
            )
          )}
        </div>
      </div>
      {fileEdit && busy && (
        <p className="px-4 py-2 text-xs text-text-muted">
          The agent is working. Save once it finishes so neither edit is lost.
        </p>
      )}
      {confirmingClose && (
        <div
          role="alertdialog"
          aria-label="Unsaved changes"
          className="flex items-center gap-2 border-t border-border px-4 py-2 text-xs"
        >
          <span className="flex-1 text-text-primary">Discard your unsaved changes?</span>
          <Button
            variant="ghost"
            className="h-7 px-2 text-xs"
            onClick={() => setConfirmingClose(false)}
          >
            Keep editing
          </Button>
          <Button variant="danger" className="h-7 px-2 text-xs" onClick={onClose}>
            Discard
          </Button>
        </div>
      )}
      {(error || saveError) && (
        <p role="alert" className="px-4 py-2 text-xs text-danger select-text">
          {saveError ?? error}
        </p>
      )}
      {footer && history.length === 1 && (
        <footer className="flex flex-wrap items-center justify-end gap-2 border-t border-border px-4 py-2.5">
          {footer}
        </footer>
      )}
    </section>
  );
}
