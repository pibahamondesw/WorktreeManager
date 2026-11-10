import { RefObject, useEffect, useLayoutEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

interface DropPoint {
  x: number;
  y: number;
}

/**
 * On macOS the native drop handler reports positions in view points, which are CSS pixels.
 * Hit-testing the topmost element keeps overlays such as modals from passing drops through.
 */
function contains(element: HTMLElement | null, { x, y }: DropPoint) {
  const hit = element ? document.elementFromPoint(x, y) : null;
  return !!hit && element!.contains(hit);
}

/**
 * Files dragged in from Finder, delivered by Tauri's native handler so their paths are known.
 * Returns whether files are hovering over the element.
 */
export function useFileDrop(
  ref: RefObject<HTMLElement | null>,
  onDrop: (paths: string[]) => void,
  enabled = true
) {
  const [hovering, setHovering] = useState(false);
  const onDropRef = useRef(onDrop);
  useLayoutEffect(() => {
    onDropRef.current = onDrop;
  });

  useEffect(() => {
    if (!enabled) return;
    let draggingFiles = false;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (payload.type === "leave") {
          draggingFiles = false;
          setHovering(false);
          return;
        }
        if (payload.type === "enter") draggingFiles = payload.paths.length > 0;
        const inside = draggingFiles && contains(ref.current, payload.position);
        if (payload.type === "drop") {
          draggingFiles = false;
          setHovering(false);
          if (inside) onDropRef.current(payload.paths);
        } else setHovering(inside);
      })
      .then((stop) => (disposed ? stop() : (unlisten = stop)))
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
      setHovering(false);
    };
  }, [ref, enabled]);

  return hovering;
}
