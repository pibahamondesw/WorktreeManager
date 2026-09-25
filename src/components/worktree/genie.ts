// Port of the slice-and-trapezoid technique from BCGenieEffect (MIT, Bartosz Ciechanowski):
// the view is cut into vertical strips, two S-shaped Bézier curves outline the funnel, and each
// strip is projected onto the trapezoid the curves bound at its position.

export interface Point {
  x: number;
  y: number;
}

export type Quad = [Point, Point, Point, Point];

export interface GenieGeometry {
  width: number;
  height: number;
  neck: { x: number; top: number; bottom: number; depth: number };
  sliceWidth: number;
}

const CURVES_END = 0.4;
const SLIDE_START = 0.3;

const clamp01 = (value: number) => Math.min(1, Math.max(0, value));
const ease = (t: number, from: number, to: number) => from + t * t * (3 - 2 * t) * (to - from);
const phase = (start: number, end: number, t: number) => clamp01((t - start) / (end - start));

interface Curve {
  from: Point;
  to: Point;
}

function curveYAt(curve: Curve, x: number): number {
  const midX = (curve.from.x + curve.to.x) / 2;
  const cubic = (t: number, a: number, b: number, c: number, d: number) => {
    const n = 1 - t;
    return n * n * n * a + 3 * n * n * t * b + 3 * n * t * t * c + t * t * t * d;
  };
  const span = curve.to.x - curve.from.x;
  let t = span === 0 ? 0 : clamp01((x - curve.from.x) / span);
  for (let i = 0; i < 3; i++) {
    const n = 1 - t;
    const fx = cubic(t, curve.from.x, midX, midX, curve.to.x) - x;
    const dfx = 3 * n * n * (midX - curve.from.x) + 3 * t * t * (curve.to.x - midX);
    if (Math.abs(dfx) < 1e-9) break;
    t = clamp01(t - fx / dfx);
  }
  return cubic(t, curve.from.y, curve.from.y, curve.to.y, curve.to.y);
}

export function sliceCount({ width, sliceWidth }: GenieGeometry) {
  return Math.ceil(width / sliceWidth);
}

export function genieQuads(geometry: GenieGeometry, progress: number): Quad[] {
  const { width, height, neck, sliceWidth } = geometry;
  const curveProgress = phase(0, CURVES_END, progress);
  const top: Curve = {
    from: { x: 0, y: 0 },
    to: { x: neck.x, y: ease(curveProgress, 0, neck.top) },
  };
  const bottom: Curve = {
    from: { x: 0, y: height },
    to: { x: neck.x, y: ease(curveProgress, height, neck.bottom) },
  };
  const edgeAt = (x: number): [Point, Point] => {
    if (x <= neck.x) {
      const clamped = Math.max(x, 0);
      return [
        { x, y: curveYAt(top, clamped) },
        { x, y: curveYAt(bottom, clamped) },
      ];
    }
    const squeeze = ease(curveProgress, 1, neck.depth / width);
    const compressedX = neck.x + (x - neck.x) * squeeze;
    return [
      { x: compressedX, y: top.to.y },
      { x: compressedX, y: bottom.to.y },
    ];
  };

  let position = ease(phase(SLIDE_START, 1, progress), 0, neck.x);
  let [leadingTop, leadingBottom] = edgeAt(position);
  return Array.from({ length: sliceCount(geometry) }, (_, index) => {
    const end = position + Math.min(sliceWidth, width - index * sliceWidth);
    const [trailingTop, trailingBottom] = edgeAt(end);
    const quad: Quad = [leadingTop, trailingTop, trailingBottom, leadingBottom];
    [leadingTop, leadingBottom, position] = [trailingTop, trailingBottom, end];
    return quad;
  });
}

// Projective map of the rect (0,0)-(w,h) onto quad [topLeft, topRight, bottomRight, bottomLeft].
export function rectToQuadMatrix(w: number, h: number, [p0, p1, p2, p3]: Quad): string {
  const dx1 = p1.x - p2.x;
  const dx2 = p3.x - p2.x;
  const dx3 = p0.x - p1.x + p2.x - p3.x;
  const dy1 = p1.y - p2.y;
  const dy2 = p3.y - p2.y;
  const dy3 = p0.y - p1.y + p2.y - p3.y;
  const det = dx1 * dy2 - dx2 * dy1;
  const g = Math.abs(det) < 1e-9 ? 0 : (dx3 * dy2 - dx2 * dy3) / det;
  const hh = Math.abs(det) < 1e-9 ? 0 : (dx1 * dy3 - dx3 * dy1) / det;
  const a = p1.x - p0.x + g * p1.x;
  const b = p3.x - p0.x + hh * p3.x;
  const d = p1.y - p0.y + g * p1.y;
  const e = p3.y - p0.y + hh * p3.y;
  const values = [a / w, d / w, 0, g / w, b / h, e / h, 0, hh / h, 0, 0, 1, 0, p0.x, p0.y, 0, 1];
  return `matrix3d(${values.map((value) => (Number.isFinite(value) ? value : 0)).join(",")})`;
}
