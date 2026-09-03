import { useLayoutEffect, useState } from "react";

const STORAGE_KEY = "worktreemanager.uiZoom";
const MIN_ZOOM = 80;
const MAX_ZOOM = 150;

function readZoom(): number {
  try {
    const value = Number(localStorage.getItem(STORAGE_KEY));
    return Number.isInteger(value) && value >= MIN_ZOOM && value <= MAX_ZOOM ? value : 100;
  } catch {
    return 100;
  }
}

export function useZoom() {
  const [zoom, setZoom] = useState(readZoom);

  useLayoutEffect(() => {
    const root = document.documentElement;
    const previous = root.style.fontSize;
    root.style.fontSize = `${16 * (zoom / 100)}px`;
    try {
      localStorage.setItem(STORAGE_KEY, String(zoom));
    } catch {
      // Zoom remains usable when storage is unavailable.
    }
    return () => {
      root.style.fontSize = previous;
    };
  }, [zoom]);

  useLayoutEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.isComposing) return;
      const increase = event.key === "+" || event.key === "=";
      const decrease = event.key === "-" || event.code === "NumpadSubtract";
      if (!increase && !decrease) return;

      // Capture also covers inputs and modals. Only zoom keys are consumed.
      event.preventDefault();
      event.stopImmediatePropagation();
      setZoom((current) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, current + (increase ? 10 : -10))));
    };
    window.addEventListener("keydown", handleKeyDown, true);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, []);

  return {
    zoom,
    zoomIn: () => setZoom((current) => Math.min(MAX_ZOOM, current + 10)),
    zoomOut: () => setZoom((current) => Math.max(MIN_ZOOM, current - 10)),
    resetZoom: () => setZoom(100),
    canZoomIn: zoom < MAX_ZOOM,
    canZoomOut: zoom > MIN_ZOOM,
  };
}
