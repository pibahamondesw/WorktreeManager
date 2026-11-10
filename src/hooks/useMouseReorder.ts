import { MouseEvent as ReactMouseEvent, useId, useLayoutEffect, useRef, useState } from "react";

const DRAG_THRESHOLD_PX = 4;

function suppressNextClick() {
  const swallow = (event: MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
  };
  window.addEventListener("click", swallow, { capture: true, once: true });
  setTimeout(() => window.removeEventListener("click", swallow, { capture: true }));
}

/**
 * Mouse-driven list reordering. HTML5 drag and drop can't be used because the native file drop
 * handler claims every drag over the webview.
 */
export function useMouseReorder(onReorder: (source: string, target: string) => void) {
  const group = useId();
  const [dragging, setDragging] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const onReorderRef = useRef(onReorder);
  useLayoutEffect(() => {
    onReorderRef.current = onReorder;
  });

  const targetUnder = (event: MouseEvent) => {
    const element = event.target instanceof Element ? event.target : null;
    const item = element?.closest<HTMLElement>("[data-reorder-key]");
    return item?.dataset.reorderGroup === group ? (item.dataset.reorderKey ?? null) : null;
  };

  const start = (key: string, event: ReactMouseEvent) => {
    if (event.button !== 0) return;
    const origin = { x: event.clientX, y: event.clientY };
    const previousUserSelect = document.body.style.userSelect;
    let active = false;
    let target: string | null = null;

    const move = (moved: MouseEvent) => {
      if (!active) {
        const distance = Math.hypot(moved.clientX - origin.x, moved.clientY - origin.y);
        if (distance < DRAG_THRESHOLD_PX) return;
        active = true;
        window.getSelection()?.removeAllRanges();
        document.body.style.userSelect = "none";
        setDragging(key);
      }
      moved.preventDefault();
      target = targetUnder(moved);
      setOver(target);
    };

    const end = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", end);
      if (!active) return;
      document.body.style.userSelect = previousUserSelect;
      suppressNextClick();
      setDragging(null);
      setOver(null);
      if (target && target !== key) onReorderRef.current(key, target);
    };

    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", end);
  };

  const itemProps = (key: string, enabled = true) =>
    enabled
      ? {
          "data-reorder-group": group,
          "data-reorder-key": key,
          onMouseDown: (event: ReactMouseEvent) => start(key, event),
        }
      : {};

  return { dragging, over, itemProps };
}
