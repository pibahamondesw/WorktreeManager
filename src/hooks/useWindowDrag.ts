import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Replaces Tauri's built-in `data-tauri-drag-region` handler, which is unreliable on macOS:
 * it only matches the exact event target (not ancestors) and gates on `event.detail`, which
 * keeps incrementing because the webview never sees the mouseup that ends a native drag —
 * so the click after a drag is read as a double click and dragging silently stops working.
 */
const DRAG_ATTR = "data-drag-region";
const INTERACTIVE_SELECTOR =
  "button, input, textarea, select, a, [contenteditable], [data-no-drag]";
const DOUBLE_CLICK_MS = 400;

export function useWindowDrag() {
  useEffect(() => {
    let lastPressAt = 0;

    const handleMouseDown = (event: MouseEvent) => {
      if (event.button !== 0) return;
      const target = event.target instanceof Element ? event.target : null;
      if (!target?.closest(`[${DRAG_ATTR}]`)) return;
      if (target.closest(INTERACTIVE_SELECTOR)) return;

      event.preventDefault();
      const now = Date.now();
      const isDoubleClick = now - lastPressAt < DOUBLE_CLICK_MS;
      lastPressAt = isDoubleClick ? 0 : now;

      const appWindow = getCurrentWindow();
      void (isDoubleClick ? appWindow.toggleMaximize() : appWindow.startDragging());
    };

    document.addEventListener("mousedown", handleMouseDown);
    return () => document.removeEventListener("mousedown", handleMouseDown);
  }, []);
}
