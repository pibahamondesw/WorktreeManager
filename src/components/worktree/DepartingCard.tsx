import { ReactNode, useLayoutEffect, useRef } from "react";
import { TrashIcon } from "../ui/Icons";
import { GenieGeometry, Point, genieQuads, rectToQuadMatrix, sliceCount } from "./genie";

const GENIE_MS = 560;
const GULP_MS = 180;
const COLLAPSE_MS = 200;
const MAX_SLICES = 60;
const MIN_SLICE_WIDTH = 8;
const NECK_INSET = 6;
const EXIT_EASING = "cubic-bezier(0.4, 0, 1, 1)";
const SETTLE_EASING = "cubic-bezier(0.16, 1, 0.3, 1)";

function prefersReducedMotion() {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

function boxWithin(element: HTMLElement, container: HTMLElement): Box {
  const rect = element.getBoundingClientRect();
  const origin = container.getBoundingClientRect();
  return {
    left: rect.left - origin.left,
    top: rect.top - origin.top,
    width: rect.width,
    height: rect.height,
  };
}

function centerOf(box: Box): Point {
  return { x: box.left + box.width / 2, y: box.top + box.height / 2 };
}

function genieGeometry(card: HTMLElement, trash: Box): GenieGeometry {
  const width = card.offsetWidth;
  return {
    width,
    height: card.offsetHeight,
    neck: {
      x: trash.left + NECK_INSET,
      top: trash.top + NECK_INSET,
      bottom: trash.top + trash.height - NECK_INSET,
      depth: trash.width - 2 * NECK_INSET,
    },
    sliceWidth: Math.max(MIN_SLICE_WIDTH, Math.ceil(width / MAX_SLICES)),
  };
}

function sliceWidthAt(geometry: GenieGeometry, index: number) {
  return Math.min(geometry.sliceWidth, geometry.width - index * geometry.sliceWidth);
}

function buildSlices(card: HTMLElement, layer: HTMLElement, geometry: GenieGeometry) {
  return Array.from({ length: sliceCount(geometry) }, (_, index) => {
    const slice = document.createElement("div");
    Object.assign(slice.style, {
      position: "absolute",
      left: "0",
      top: "0",
      width: `${sliceWidthAt(geometry, index) + 0.5}px`,
      height: `${geometry.height}px`,
      overflow: "hidden",
      transformOrigin: "0 0",
      backfaceVisibility: "hidden",
    });
    const copy = card.cloneNode(true) as HTMLElement;
    Object.assign(copy.style, {
      position: "absolute",
      left: `${-index * geometry.sliceWidth}px`,
      top: "0",
      width: `${geometry.width}px`,
    });
    slice.append(copy);
    layer.append(slice);
    return slice;
  });
}

function renderSlices(slices: HTMLElement[], geometry: GenieGeometry, progress: number) {
  genieQuads(geometry, progress).forEach((quad, index) => {
    slices[index].style.transform = rectToQuadMatrix(
      sliceWidthAt(geometry, index),
      geometry.height,
      quad
    );
  });
}

export function DepartingCard({
  taskId,
  children,
  gap,
  onDeparted,
}: {
  taskId: string;
  children: ReactNode;
  gap: number;
  onDeparted: (taskId: string) => void;
}) {
  const rowRef = useRef<HTMLDivElement>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  const layerRef = useRef<HTMLDivElement>(null);
  const trashRef = useRef<HTMLSpanElement>(null);

  useLayoutEffect(() => {
    const row = rowRef.current;
    const card = cardRef.current;
    const layer = layerRef.current;
    const trash = trashRef.current;
    const target = card?.querySelector<HTMLElement>("[data-genie-target]");
    if (
      !row ||
      !card ||
      !layer ||
      !trash ||
      !target ||
      typeof row.animate !== "function" ||
      prefersReducedMotion()
    ) {
      onDeparted(taskId);
      return;
    }
    const trashBox = boxWithin(target, card);
    const trashCenter = centerOf(trashBox);
    const geometry = genieGeometry(card, trashBox);
    const slices = buildSlices(card, layer, geometry);
    renderSlices(slices, geometry, 0);
    card.style.visibility = "hidden";
    Object.assign(trash.style, { left: `${trashCenter.x}px`, top: `${trashCenter.y}px` });
    trash.hidden = false;

    const animations: Animation[] = [];
    let frame = 0;
    const startedAt = performance.now();
    const finish = () => {
      layer.replaceChildren();
      const gulp = trash.animate(
        [
          { scale: "1", opacity: 1 },
          { scale: "1.25", opacity: 1, offset: 0.4 },
          { scale: "0.6", opacity: 0 },
        ],
        { duration: GULP_MS, easing: EXIT_EASING, fill: "forwards" }
      );
      const collapse = row.animate(
        [
          { height: `${row.offsetHeight}px`, marginBottom: "0px" },
          { height: "0px", marginBottom: `${-gap}px` },
        ],
        { duration: COLLAPSE_MS, delay: GULP_MS / 2, easing: SETTLE_EASING, fill: "forwards" }
      );
      animations.push(gulp, collapse);
      collapse.onfinish = () => onDeparted(taskId);
    };
    const tick = (now: number) => {
      const progress = Math.min(1, (now - startedAt) / GENIE_MS);
      renderSlices(slices, geometry, progress);
      if (progress < 1) frame = requestAnimationFrame(tick);
      else finish();
    };
    frame = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(frame);
      animations.forEach((animation) => animation.cancel());
      layer.replaceChildren();
      card.style.visibility = "";
    };
  }, [taskId, gap, onDeparted]);

  return (
    <div ref={rowRef} inert className="relative">
      <div ref={cardRef} className="[&_[data-genie-target]]:invisible">
        {children}
      </div>
      <div ref={layerRef} aria-hidden="true" className="absolute inset-0 pointer-events-none" />
      <span
        ref={trashRef}
        hidden
        aria-hidden="true"
        style={{ transform: "translate(-50%, -50%)" }}
        className="absolute z-10 w-8 h-8 flex items-center justify-center rounded-lg text-danger bg-danger/10"
      >
        <TrashIcon />
      </span>
    </div>
  );
}
