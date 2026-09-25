import { ReactNode, useLayoutEffect, useRef } from "react";

const GENIE_MS = 320;
const COLLAPSE_MS = 200;
const EXIT_EASING = "cubic-bezier(0.4, 0, 1, 1)";
const SETTLE_EASING = "cubic-bezier(0.16, 1, 0.3, 1)";

function prefersReducedMotion() {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

function genieOrigin(card: HTMLElement) {
  const target = card.querySelector<HTMLElement>("[data-genie-target]");
  if (!target) return "100% 0%";
  const cardRect = card.getBoundingClientRect();
  const targetRect = target.getBoundingClientRect();
  const x = targetRect.left + targetRect.width / 2 - cardRect.left;
  const y = targetRect.top + targetRect.height / 2 - cardRect.top;
  return `${x}px ${y}px`;
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

  useLayoutEffect(() => {
    const row = rowRef.current;
    const card = cardRef.current;
    if (!row || !card || typeof card.animate !== "function" || prefersReducedMotion()) {
      onDeparted(taskId);
      return;
    }
    card.style.transformOrigin = genieOrigin(card);
    const genie = card.animate(
      [
        { transform: "scale(1, 1)", opacity: 1 },
        { transform: "scale(0.45, 0.85)", opacity: 0.9, offset: 0.45 },
        { transform: "scale(0.04, 0.08)", opacity: 0 },
      ],
      { duration: GENIE_MS, easing: EXIT_EASING, fill: "forwards" }
    );
    let collapse: Animation | undefined;
    genie.onfinish = () => {
      collapse = row.animate(
        [
          { height: `${row.offsetHeight}px`, marginBottom: "0px" },
          { height: "0px", marginBottom: `${-gap}px` },
        ],
        { duration: COLLAPSE_MS, easing: SETTLE_EASING, fill: "forwards" }
      );
      collapse.onfinish = () => onDeparted(taskId);
    };
    return () => {
      genie.cancel();
      collapse?.cancel();
    };
  }, [taskId, gap, onDeparted]);

  return (
    <div ref={rowRef} inert className="overflow-visible">
      <div ref={cardRef}>{children}</div>
    </div>
  );
}
