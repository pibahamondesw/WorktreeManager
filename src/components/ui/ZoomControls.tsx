import { useZoom } from "../../hooks/useZoom";

/** Lives in the fixed native-titlebar strip, including during setup and in modals. */
export function ZoomControls() {
  const { zoom, zoomIn, zoomOut, resetZoom, canZoomIn, canZoomOut } = useZoom();
  const buttonClass =
    "px-1 rounded hover:bg-bg-hover hover:text-text-primary disabled:opacity-30 disabled:cursor-default cursor-pointer focus-visible:outline focus-visible:outline-accent";

  return (
    <div
      role="group"
      aria-label="Interface zoom"
      data-no-drag
      className="fixed top-0 right-2 h-[32px] z-[60] flex items-center gap-1 text-xs text-text-secondary"
    >
      <button
        type="button"
        className={buttonClass}
        onClick={zoomOut}
        disabled={!canZoomOut}
        aria-label="Zoom out"
        title="Zoom out (⌘− / Ctrl−)"
      >
        −
      </button>
      <button
        type="button"
        className={`${buttonClass} tabular-nums`}
        onClick={resetZoom}
        aria-label={`Zoom ${zoom}%. Reset zoom`}
        title="Reset zoom to 100%"
      >
        <span role="status" aria-live="polite">
          {zoom}%
        </span>
      </button>
      <button
        type="button"
        className={buttonClass}
        onClick={zoomIn}
        disabled={!canZoomIn}
        aria-label="Zoom in"
        title="Zoom in (⌘+ / Ctrl+)"
      >
        +
      </button>
    </div>
  );
}
