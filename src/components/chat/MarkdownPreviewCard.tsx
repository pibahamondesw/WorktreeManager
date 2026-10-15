import { KeyboardEvent, useMemo } from "react";
import { DocumentIcon, ExpandIcon } from "../ui/Icons";
import {
  documentSummary,
  fileName,
  markdownReferences,
  MarkdownSource,
  relativeTo,
} from "./markdownDocument";
import { useMarkdownDocument, useMarkdownPreview } from "./markdownPreviewContext";

const hasSelection = () => !(window.getSelection()?.isCollapsed ?? true);

function MarkdownPreviewCard({
  source,
  badge,
  requestId,
}: {
  source: MarkdownSource;
  badge?: string;
  requestId?: string;
}) {
  const preview = useMarkdownPreview();
  const folders = preview?.folders ?? [];
  const loaded = useMarkdownDocument(source, folders);
  const text = loaded.kind === "ready" ? loaded.text : "";
  const summary = useMemo(() => documentSummary(text), [text]);
  if (!preview || loaded.kind !== "ready") return null;

  const path = loaded.document?.path;
  const title =
    summary.title ?? (path ? fileName(path) : source.kind === "inline" ? source.title : "");
  const subtitle = path ? relativeTo(path, folders) : null;
  const open = () => preview.open({ source, requestId });
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.target !== event.currentTarget || (event.key !== "Enter" && event.key !== " "))
      return;
    event.preventDefault();
    open();
  };

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={`Preview ${title}`}
      onClick={() => !hasSelection() && open()}
      onKeyDown={onKeyDown}
      className="markdown-card group relative min-w-0 overflow-hidden rounded-lg border border-border bg-bg-secondary/60 text-left cursor-pointer transition-colors hover:border-border-light hover:bg-bg-secondary focus-visible:outline-none focus-visible:border-accent"
    >
      <div className="flex items-start gap-2.5 px-3 pt-2.5">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md bg-accent/12 text-accent-hover">
          <DocumentIcon size={15} />
        </span>
        <div className="flex-1 min-w-0">
          <div className="truncate text-sm font-medium text-text-primary">{title}</div>
          {subtitle && (
            <div
              title={path}
              className="truncate font-mono text-[11px] text-text-muted select-text cursor-text"
            >
              {subtitle}
            </div>
          )}
        </div>
        {badge && (
          <span className="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide bg-accent/15 text-accent-hover">
            {badge}
          </span>
        )}
      </div>
      {summary.lines.length > 0 ? (
        <div aria-hidden="true" className="markdown-card-excerpt px-3 pt-2 pb-2.5">
          {summary.lines.map((line, index) => (
            <p key={index} className="truncate text-xs leading-5 text-text-secondary">
              {line}
            </p>
          ))}
        </div>
      ) : (
        <p className="px-3 pt-1.5 pb-2.5 text-xs italic text-text-muted">Empty document</p>
      )}
      <span className="markdown-card-hint pointer-events-none absolute right-2.5 bottom-2 flex items-center gap-1.5 rounded-md border border-border bg-bg-tertiary px-2 py-1 text-[11px] text-text-secondary">
        <ExpandIcon size={11} />
        Open preview
      </span>
    </div>
  );
}

export function ReferencedDocuments({ text }: { text: string }) {
  const paths = useMemo(() => markdownReferences(text), [text]);
  if (paths.length === 0) return null;
  return (
    <div className="grid gap-2 grid-cols-[repeat(auto-fit,minmax(min(100%,18rem),1fr))] empty:hidden">
      {paths.map((path) => (
        <MarkdownPreviewCard key={path} source={{ kind: "file", path }} />
      ))}
    </div>
  );
}

export function PlanCard({ text, requestId }: { text: string; requestId?: string }) {
  return (
    <MarkdownPreviewCard
      source={{ kind: "inline", title: "Plan", text }}
      badge="Plan"
      requestId={requestId}
    />
  );
}
