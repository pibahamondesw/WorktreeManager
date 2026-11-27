import { KeyboardEvent as ReactKeyboardEvent, RefObject, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CloseIcon, ExpandIcon } from "../ui/Icons";
import { useEditorOcclusion } from "../../hooks/useEditorOcclusion";
import { useThemeRevision } from "../../hooks/useThemeRevision";
import {
  HTML_PREVIEW_ESCAPE_MESSAGE,
  HtmlPreviewFrameSource,
  htmlPreviewFrameSource,
} from "./htmlPreviewPage";
import { VIEWER_TOOLBAR_BUTTON } from "./viewerToolbar";

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 2;
const ZOOM_STEP = 0.25;

function HtmlPreviewFrame({
  frame,
  className,
  ref,
}: {
  frame: HtmlPreviewFrameSource;
  className: string;
  ref?: RefObject<HTMLIFrameElement | null>;
}) {
  return (
    <iframe
      key={"src" in frame ? frame.src : frame.srcDoc}
      ref={ref}
      tabIndex={"src" in frame ? undefined : -1}
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
      <figcaption className="chat-html-preview-bar">
        <span>HTML preview</span>
        <button
          type="button"
          className="chat-mermaid-action"
          onClick={() => setFullScreen(true)}
          aria-label="Open HTML preview full screen"
          title="Open HTML preview full screen"
        >
          <ExpandIcon />
        </button>
      </figcaption>
      <HtmlPreviewFrame frame={frame} className="chat-html-preview-frame" />
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
  const frameRef = useRef<HTMLIFrameElement>(null);
  useEscapeFromFrame(frameRef, onClose);
  useStaticFrameReturnsFocus(frame, frameRef, dialogRef);
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
          <HtmlPreviewFrame frame={frame} className="block size-full" ref={frameRef} />
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

function useEscapeFromFrame(frameRef: RefObject<HTMLIFrameElement | null>, onEscape: () => void) {
  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      const fromFrame = !!event.source && event.source === frameRef.current?.contentWindow;
      if (fromFrame && event.data === HTML_PREVIEW_ESCAPE_MESSAGE) onEscape();
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [frameRef, onEscape]);
}

/** A static page can't forward Escape, so the dialog takes focus back when the page takes it. */
function useStaticFrameReturnsFocus(
  frame: HtmlPreviewFrameSource,
  frameRef: RefObject<HTMLIFrameElement | null>,
  dialogRef: RefObject<HTMLDivElement | null>
) {
  const isStatic = !("src" in frame);
  useEffect(() => {
    if (!isStatic) return;
    let timer: number | undefined;
    const onBlur = () => {
      timer = window.setTimeout(() => {
        if (document.activeElement === frameRef.current) dialogRef.current?.focus();
      });
    };
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("blur", onBlur);
      window.clearTimeout(timer);
    };
  }, [isStatic, frameRef, dialogRef]);
}
