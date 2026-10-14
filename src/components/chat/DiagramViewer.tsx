import {
  KeyboardEvent as ReactKeyboardEvent,
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
  useEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { CloseIcon, FitIcon, FitWidthIcon } from "../ui/Icons";
import { useEditorOcclusion } from "../../hooks/useEditorOcclusion";
import {
  MAX_SCALE,
  MIN_SCALE,
  Point,
  Size,
  View,
  ZOOM_SLIDER_MAX,
  ZOOM_SLIDER_MIN,
  ZOOM_STEP,
  fitView,
  fitWidthView,
  scaleToSlider,
  sliderToScale,
  panBy,
  zoomAt,
} from "./diagramView";

const FIT_PADDING = 64;
const DRAG_THRESHOLD = 3;
const WHEEL_ZOOM_SPEED = 0.01;
const MAX_WHEEL_ZOOM_DELTA = 20;
const KEY_PAN_STEP = 48;

interface GestureEvent extends UIEvent {
  scale: number;
  clientX: number;
  clientY: number;
}

interface Drag {
  pointerId: number;
  last: Point;
  moved: boolean;
  onBackdrop: boolean;
}

const windowSize = (): Size => ({ width: window.innerWidth, height: window.innerHeight });
const windowCenter = (): Point => ({ x: window.innerWidth / 2, y: window.innerHeight / 2 });
const fitToWindow = (size: Size) => fitView(size, windowSize(), FIT_PADDING);
const fitWidthToWindow = (size: Size) => fitWidthView(size, windowSize(), FIT_PADDING);

export function DiagramViewer({
  svg,
  size,
  label,
  onClose,
}: {
  svg: string;
  size: Size;
  label: string;
  onClose: () => void;
}) {
  useEditorOcclusion(true);
  const dialogRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<Drag | null>(null);
  const [view, setView] = useState(() => fitToWindow(size));
  const [animated, setAnimated] = useState(false);
  const [dragging, setDragging] = useState(false);

  const update = (next: (current: View) => View, animate = false) => {
    setAnimated(animate);
    setView(next);
  };
  const zoomBy = (factor: number, anchor = windowCenter()) =>
    update((current) => zoomAt(current, current.scale * factor, anchor), true);
  const zoomTo = (scale: number, animate = true) =>
    update((current) => zoomAt(current, scale, windowCenter()), animate);
  const fit = () => update(() => fitToWindow(size), true);
  const fitWidth = () => update(() => fitWidthToWindow(size), true);
  const actualSize = () => zoomTo(1);

  useEffect(() => {
    const returnFocus = document.activeElement;
    dialogRef.current?.focus();
    return () => {
      if (returnFocus instanceof HTMLElement) returnFocus.focus();
    };
  }, []);

  useEffect(() => {
    const surface = dialogRef.current;
    if (!surface) return;
    let gestureScale = 1;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      setAnimated(false);
      if (event.ctrlKey || event.metaKey) {
        const delta = Math.max(-MAX_WHEEL_ZOOM_DELTA, Math.min(MAX_WHEEL_ZOOM_DELTA, event.deltaY));
        const factor = Math.exp(-delta * WHEEL_ZOOM_SPEED);
        const anchor = { x: event.clientX, y: event.clientY };
        setView((current) => zoomAt(current, current.scale * factor, anchor));
      } else {
        setView((current) => panBy(current, -event.deltaX, -event.deltaY));
      }
    };
    const onGestureStart = (event: Event) => {
      event.preventDefault();
      gestureScale = 1;
    };
    const onGestureChange = (event: Event) => {
      event.preventDefault();
      const gesture = event as GestureEvent;
      const factor = gesture.scale / gestureScale;
      gestureScale = gesture.scale;
      const anchor = { x: gesture.clientX, y: gesture.clientY };
      setAnimated(false);
      setView((current) => zoomAt(current, current.scale * factor, anchor));
    };
    surface.addEventListener("wheel", onWheel, { passive: false });
    surface.addEventListener("gesturestart", onGestureStart);
    surface.addEventListener("gesturechange", onGestureChange);
    return () => {
      surface.removeEventListener("wheel", onWheel);
      surface.removeEventListener("gesturestart", onGestureStart);
      surface.removeEventListener("gesturechange", onGestureChange);
    };
  }, []);

  const onKeyDown = (event: ReactKeyboardEvent) => {
    if (event.metaKey || event.ctrlKey || event.altKey || event.nativeEvent.isComposing) return;
    if (event.target instanceof HTMLInputElement && event.key !== "Escape") return;
    const actions: Record<string, () => void> = {
      Escape: onClose,
      "+": () => zoomBy(ZOOM_STEP),
      "=": () => zoomBy(ZOOM_STEP),
      "-": () => zoomBy(1 / ZOOM_STEP),
      "0": fit,
      "1": actualSize,
      w: fitWidth,
      ArrowLeft: () => update((current) => panBy(current, KEY_PAN_STEP, 0), true),
      ArrowRight: () => update((current) => panBy(current, -KEY_PAN_STEP, 0), true),
      ArrowUp: () => update((current) => panBy(current, 0, KEY_PAN_STEP), true),
      ArrowDown: () => update((current) => panBy(current, 0, -KEY_PAN_STEP), true),
    };
    const action = actions[event.key];
    if (!action) return;
    event.preventDefault();
    event.stopPropagation();
    action();
  };

  const onPointerDown = (event: ReactPointerEvent) => {
    if (event.button !== 0 || (event.target as Element).closest("button, [role='toolbar']")) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = {
      pointerId: event.pointerId,
      last: { x: event.clientX, y: event.clientY },
      moved: false,
      onBackdrop: !contentRef.current?.contains(event.target as Node),
    };
  };

  const onPointerMove = (event: ReactPointerEvent) => {
    const drag = dragRef.current;
    if (drag?.pointerId !== event.pointerId) return;
    const dx = event.clientX - drag.last.x;
    const dy = event.clientY - drag.last.y;
    if (!drag.moved && Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
    drag.moved = true;
    drag.last = { x: event.clientX, y: event.clientY };
    setDragging(true);
    update((current) => panBy(current, dx, dy));
  };

  const onPointerUp = (event: ReactPointerEvent) => {
    const drag = dragRef.current;
    if (drag?.pointerId !== event.pointerId) return;
    dragRef.current = null;
    setDragging(false);
    if (!drag.moved && drag.onBackdrop) onClose();
  };

  const onDoubleClick = (event: ReactMouseEvent) => {
    if (!contentRef.current?.contains(event.target as Node)) return;
    const fitted = fitToWindow(size);
    if (view.scale > fitted.scale * 1.01) update(() => fitted, true);
    else zoomBy(2, { x: event.clientX, y: event.clientY });
  };

  const zoomPercent = Math.round(view.scale * 100);
  const toolbarButton =
    "h-8 min-w-8 px-2 rounded-md flex items-center justify-center text-text-secondary hover:bg-bg-hover hover:text-text-primary disabled:opacity-30 disabled:cursor-default cursor-pointer focus-visible:outline focus-visible:outline-accent";

  return createPortal(
    <div
      ref={dialogRef}
      role="dialog"
      aria-modal="true"
      aria-label={label}
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onDoubleClick={onDoubleClick}
      className={`diagram-viewer fixed inset-0 z-50 overflow-hidden bg-bg-primary/95 backdrop-blur-sm motion-fade outline-none select-none ${
        dragging ? "cursor-grabbing" : "cursor-grab"
      }`}
    >
      <div
        ref={contentRef}
        className={`diagram-viewer-content absolute left-0 top-0 origin-top-left ${
          animated ? "transition-transform duration-150 ease-out" : ""
        }`}
        style={{
          width: size.width,
          height: size.height,
          transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})`,
        }}
        dangerouslySetInnerHTML={{ __html: svg }}
      />
      <button
        type="button"
        onClick={onClose}
        aria-label="Close diagram preview"
        title="Close (Esc)"
        className={`${toolbarButton} absolute top-10 right-4 bg-bg-secondary border border-border shadow-lg`}
      >
        <CloseIcon size={14} />
      </button>
      <div
        role="toolbar"
        aria-label="Diagram zoom"
        className="absolute bottom-6 left-1/2 -translate-x-1/2 flex items-center gap-0.5 p-1 rounded-lg bg-bg-secondary border border-border shadow-lg text-sm cursor-default"
      >
        <button
          type="button"
          className={toolbarButton}
          onClick={() => zoomBy(1 / ZOOM_STEP)}
          disabled={view.scale <= MIN_SCALE}
          aria-label="Zoom out"
          title="Zoom out (−)"
        >
          −
        </button>
        <input
          type="range"
          aria-label="Zoom level"
          aria-valuetext={`${zoomPercent}%`}
          min={ZOOM_SLIDER_MIN}
          max={ZOOM_SLIDER_MAX}
          step={0.01}
          value={scaleToSlider(view.scale)}
          onChange={(event) => zoomTo(sliderToScale(Number(event.target.value)), false)}
          className="diagram-zoom-slider w-36 mx-1 cursor-pointer"
        />
        <button
          type="button"
          className={toolbarButton}
          onClick={() => zoomBy(ZOOM_STEP)}
          disabled={view.scale >= MAX_SCALE}
          aria-label="Zoom in"
          title="Zoom in (+)"
        >
          +
        </button>
        <span
          role="status"
          aria-live="polite"
          className="w-12 text-center text-xs tabular-nums text-text-secondary"
        >
          {zoomPercent}%
        </span>
        <span className="mx-1 h-4 w-px bg-border" aria-hidden="true" />
        <button
          type="button"
          className={toolbarButton}
          onClick={fit}
          aria-label="Fit to screen"
          title="Fit to screen (0)"
        >
          <FitIcon />
        </button>
        <button
          type="button"
          className={toolbarButton}
          onClick={fitWidth}
          aria-label="Fit to width"
          title="Fit to width (W)"
        >
          <FitWidthIcon />
        </button>
        <button
          type="button"
          className={`${toolbarButton} text-xs tabular-nums`}
          onClick={actualSize}
          aria-label="Actual size"
          title="Actual size (1)"
        >
          1:1
        </button>
      </div>
    </div>,
    document.body
  );
}
