import { KeyboardEvent as ReactKeyboardEvent, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CloseIcon, ExpandIcon } from "../ui/Icons";
import { useEditorOcclusion } from "../../hooks/useEditorOcclusion";
import { useThemeRevision } from "../../hooks/useThemeRevision";
import { HtmlPreviewFrameSource, htmlPreviewFrameSource } from "./htmlPreviewPage";
import { VIEWER_TOOLBAR_BUTTON } from "./viewerToolbar";

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 2;
const ZOOM_STEP = 0.25;

function HtmlPreviewFrame({
  frame,
  className,
}: {
  frame: HtmlPreviewFrameSource;
  className: string;
}) {
  return (
    <iframe
      key={"src" in frame ? frame.src : frame.srcDoc}
      title="HTML preview"
      referrerPolicy="no-referrer"
      className={className}
      {...frame}
    />
  );
}

export function HtmlPreview({ source }: { source: string }) {
  useThemeRevision();
  const frame = htmlPreviewFrameSource(source);
  const [fullScreen, setFullScreen] = useState(false);
  return (
    <figure className="chat-html-preview">
      <HtmlPreviewFrame frame={frame} className="chat-html-preview-frame" />
      <div className="chat-html-preview-actions">
        <button
          type="button"
          className="chat-mermaid-action"
          onClick={() => setFullScreen(true)}
          aria-label="Open HTML preview full screen"
          title="Open HTML preview full screen"
        >
          <ExpandIcon />
        </button>
      </div>
      {fullScreen && <HtmlPreviewViewer frame={frame} onClose={() => setFullScreen(false)} />}
    </figure>
  );
}

function HtmlPreviewViewer({
  frame,
  onClose,
}: {
  frame: HtmlPreviewFrameSource;
  onClose: () => void;
}) {
  useEditorOcclusion(true);
  const dialogRef = useRef<HTMLDivElement>(null);
  const [zoom, setZoom] = useState(1);
  const zoomBy = (delta: number) =>
    setZoom((current) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, current + delta)));

  useEffect(() => {
    const returnFocus = document.activeElement;
    dialogRef.current?.focus();
    return () => {
      if (returnFocus instanceof HTMLElement) returnFocus.focus();
    };
  }, []);

  const onKeyDown = (event: ReactKeyboardEvent) => {
    if (event.metaKey || event.ctrlKey || event.altKey || event.nativeEvent.isComposing) return;
    const actions: Record<string, () => void> = {
      Escape: onClose,
      "+": () => zoomBy(ZOOM_STEP),
      "=": () => zoomBy(ZOOM_STEP),
      "-": () => zoomBy(-ZOOM_STEP),
      "0": () => setZoom(1),
    };
    const action = actions[event.key];
    if (!action) return;
    event.preventDefault();
    event.stopPropagation();
    action();
  };

  const zoomPercent = Math.round(zoom * 100);
  const toolbarButton = VIEWER_TOOLBAR_BUTTON;

  return createPortal(
    <div
      ref={dialogRef}
      role="dialog"
      aria-modal="true"
      aria-label="HTML preview"
      tabIndex={-1}
      onKeyDown={onKeyDown}
      className="fixed inset-0 z-50 flex flex-col gap-3 px-6 pt-10 pb-6 bg-bg-primary/95 backdrop-blur-sm motion-fade outline-none"
    >
      <div className="relative flex-1 min-h-0 overflow-hidden rounded-lg border border-border">
        <div
          className="absolute left-0 top-0 origin-top-left"
          style={{
            width: `${100 / zoom}%`,
            height: `${100 / zoom}%`,
            transform: `scale(${zoom})`,
          }}
        >
          <HtmlPreviewFrame frame={frame} className="block size-full" />
        </div>
      </div>
      <div
        role="toolbar"
        aria-label="HTML preview zoom"
        className="self-center flex items-center gap-0.5 p-1 rounded-lg bg-bg-secondary border border-border shadow-lg text-sm"
      >
        <button
          type="button"
          className={toolbarButton}
          onClick={() => zoomBy(-ZOOM_STEP)}
          disabled={zoom <= MIN_ZOOM}
          aria-label="Zoom out"
          title="Zoom out (−)"
        >
          −
        </button>
        <button
          type="button"
          className={`${toolbarButton} w-14 text-xs tabular-nums`}
          onClick={() => setZoom(1)}
          aria-label={`Zoom ${zoomPercent}%. Reset zoom`}
          title="Reset zoom to 100% (0)"
        >
          <span role="status" aria-live="polite">
            {zoomPercent}%
          </span>
        </button>
        <button
          type="button"
          className={toolbarButton}
          onClick={() => zoomBy(ZOOM_STEP)}
          disabled={zoom >= MAX_ZOOM}
          aria-label="Zoom in"
          title="Zoom in (+)"
        >
          +
        </button>
        <span className="mx-1 h-4 w-px bg-border" aria-hidden="true" />
        <button
          type="button"
          onClick={onClose}
          aria-label="Close HTML preview full screen"
          title="Close HTML preview full screen (Esc)"
          className={toolbarButton}
        >
          <CloseIcon size={14} />
        </button>
      </div>
    </div>,
    document.body
  );
}
