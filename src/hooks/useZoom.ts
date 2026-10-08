import { useShortcutActions } from "../shortcuts/runtime";
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

  useShortcutActions({
    "zoom.in": { handler: () => setZoom((current) => Math.min(MAX_ZOOM, current + 10)) },
    "zoom.out": { handler: () => setZoom((current) => Math.max(MIN_ZOOM, current - 10)) },
  });

  return {
    zoom,
    zoomIn: () => setZoom((current) => Math.min(MAX_ZOOM, current + 10)),
    zoomOut: () => setZoom((current) => Math.max(MIN_ZOOM, current - 10)),
    resetZoom: () => setZoom(100),
    canZoomIn: zoom < MAX_ZOOM,
    canZoomOut: zoom > MIN_ZOOM,
  };
}
