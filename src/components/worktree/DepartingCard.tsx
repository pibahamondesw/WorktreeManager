import { ReactNode, useLayoutEffect, useRef } from "react";
import { TrashIcon } from "../ui/Icons";

const GENIE_MS = 520;
const GULP_MS = 180;
const COLLAPSE_MS = 200;
const EXIT_EASING = "cubic-bezier(0.4, 0, 1, 1)";
const SETTLE_EASING = "cubic-bezier(0.16, 1, 0.3, 1)";

interface Point {
  x: number;
  y: number;
}

function prefersReducedMotion() {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

const GENIE_COLUMNS = 16;
const GENIE_FRAMES = 12;
const NECK_HALF_HEIGHT = 3;

const clamp01 = (value: number) => Math.min(1, Math.max(0, value));
const smoothstep = (value: number) => value * value * (3 - 2 * value);
const lerp = (from: number, to: number, t: number) => from + (to - from) * t;

function genieShape(width: number, height: number, target: Point, progress: number) {
  const left = lerp(0, target.x, smoothstep(clamp01((progress - 0.45) / 0.55)));
  const columns = Array.from({ length: GENIE_COLUMNS + 1 }, (_, i) => {
    const x = lerp(left, width, i / GENIE_COLUMNS);
    const pinch = smoothstep(clamp01(2 * progress - 1 + x / width));
    return {
      x,
      top: lerp(0, target.y - NECK_HALF_HEIGHT, pinch),
      bottom: lerp(height, target.y + NECK_HALF_HEIGHT, pinch),
    };
  });
  const points = [
    ...columns.map(({ x, top }) => `${x}px ${top}px`),
    ...columns.reverse().map(({ x, bottom }) => `${x}px ${bottom}px`),
  ];
  return `polygon(${points.join(", ")})`;
}

function genieFrames(width: number, height: number, target: Point): Keyframe[] {
  return Array.from({ length: GENIE_FRAMES + 1 }, (_, i) => {
    const progress = i / GENIE_FRAMES;
    return {
      clipPath: genieShape(width, height, target, progress),
      opacity: progress < 0.8 ? 1 : lerp(1, 0, (progress - 0.8) / 0.2),
    };
  });
}

function centerWithin(element: HTMLElement, container: HTMLElement): Point {
  const rect = element.getBoundingClientRect();
  const origin = container.getBoundingClientRect();
  return {
    x: rect.left + rect.width / 2 - origin.left,
    y: rect.top + rect.height / 2 - origin.top,
  };
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
  const trashRef = useRef<HTMLSpanElement>(null);

  useLayoutEffect(() => {
    const row = rowRef.current;
    const card = cardRef.current;
    const trash = trashRef.current;
    const target = card?.querySelector<HTMLElement>("[data-genie-target]");
    if (
      !row ||
      !card ||
      !trash ||
      !target ||
      typeof card.animate !== "function" ||
      prefersReducedMotion()
    ) {
      onDeparted(taskId);
      return;
    }
    const center = centerWithin(target, card);
    trash.style.left = `${center.x}px`;
    trash.style.top = `${center.y}px`;
    trash.hidden = false;

    const animations: Animation[] = [];
    const genie = card.animate(genieFrames(card.offsetWidth, card.offsetHeight, center), {
      duration: GENIE_MS,
      easing: EXIT_EASING,
      fill: "forwards",
    });
    animations.push(genie);
    genie.onfinish = () => {
      const gulp = trash.animate(
        [
          { transform: "translate(-50%, -50%) scale(1)", opacity: 1 },
          { transform: "translate(-50%, -50%) scale(1.25)", opacity: 1, offset: 0.4 },
          { transform: "translate(-50%, -50%) scale(0.6)", opacity: 0 },
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
    return () => animations.forEach((animation) => animation.cancel());
  }, [taskId, gap, onDeparted]);

  return (
    <div ref={rowRef} inert className="relative">
      <div ref={cardRef} className="[&_[data-genie-target]]:invisible">
        {children}
      </div>
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
