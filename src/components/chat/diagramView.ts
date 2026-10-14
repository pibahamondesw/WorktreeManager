export interface Size {
  width: number;
  height: number;
}

export interface Point {
  x: number;
  y: number;
}

export interface View extends Point {
  scale: number;
}

export const MIN_SCALE = 0.1;
export const MAX_SCALE = 8;
export const ZOOM_STEP = 1.25;
const MAX_FIT_SCALE = 4;

const clampScale = (scale: number) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale));

export function fitView(content: Size, viewport: Size, padding: number): View {
  const scale = clampScale(
    Math.min(
      (viewport.width - 2 * padding) / content.width,
      (viewport.height - 2 * padding) / content.height,
      MAX_FIT_SCALE
    )
  );
  return {
    scale,
    x: (viewport.width - content.width * scale) / 2,
    y: (viewport.height - content.height * scale) / 2,
  };
}

export function fitWidthView(content: Size, viewport: Size, padding: number): View {
  const scale = clampScale((viewport.width - 2 * padding) / content.width);
  const height = content.height * scale;
  return {
    scale,
    x: (viewport.width - content.width * scale) / 2,
    y: height < viewport.height ? (viewport.height - height) / 2 : padding,
  };
}

export const ZOOM_SLIDER_MIN = Math.log2(MIN_SCALE);
export const ZOOM_SLIDER_MAX = Math.log2(MAX_SCALE);
export const scaleToSlider = (scale: number) => Math.log2(scale);
export const sliderToScale = (value: number) => clampScale(2 ** value);

export function zoomAt(view: View, scale: number, anchor: Point): View {
  const nextScale = clampScale(scale);
  const ratio = nextScale / view.scale;
  return {
    scale: nextScale,
    x: anchor.x - (anchor.x - view.x) * ratio,
    y: anchor.y - (anchor.y - view.y) * ratio,
  };
}

export const panBy = (view: View, dx: number, dy: number): View => ({
  ...view,
  x: view.x + dx,
  y: view.y + dy,
});
