import { expect, it } from "vitest";
import { GenieGeometry, Quad, genieQuads, rectToQuadMatrix } from "./genie";

const geometry: GenieGeometry = {
  width: 400,
  height: 120,
  neck: { x: 360, top: 20, bottom: 30, depth: 32 },
  sliceWidth: 10,
};

function project(matrix: string, x: number, y: number) {
  const m = matrix.slice("matrix3d(".length, -1).split(",").map(Number);
  const w = m[3] * x + m[7] * y + m[15];
  return { x: (m[0] * x + m[4] * y + m[12]) / w, y: (m[1] * x + m[5] * y + m[13]) / w };
}

it("maps a slice rect exactly onto its trapezoid corners", () => {
  const quad: Quad = [
    { x: 5, y: 3 },
    { x: 17, y: 9 },
    { x: 16, y: 40 },
    { x: 4, y: 52 },
  ];
  const matrix = rectToQuadMatrix(10, 60, quad);
  [
    [0, 0],
    [10, 0],
    [10, 60],
    [0, 60],
  ].forEach(([x, y], i) => {
    const point = project(matrix, x, y);
    expect(point.x).toBeCloseTo(quad[i].x, 6);
    expect(point.y).toBeCloseTo(quad[i].y, 6);
  });
});

it("starts as the untouched card", () => {
  const quads = genieQuads(geometry, 0);
  expect(quads).toHaveLength(40);
  expect(quads[0][0]).toEqual({ x: 0, y: 0 });
  expect(quads[39][2].x).toBeCloseTo(400);
  expect(quads[39][2].y).toBeCloseTo(120);
});

it("pinches the far edge into the neck before sliding the card through it", () => {
  const pinched = genieQuads(geometry, 0.4).flat();
  expect(pinched[0].x).toBeGreaterThan(0);
  pinched
    .filter((point) => point.x >= 360)
    .forEach((point) => expect(point.y >= 20 - 1e-6 && point.y <= 30 + 1e-6).toBe(true));
  expect(Math.min(...pinched.map((point) => point.y))).toBeLessThan(1);

  const swallowed = genieQuads(geometry, 1);
  swallowed.flat().forEach((point) => {
    expect(point.x).toBeGreaterThanOrEqual(360 - 1e-6);
    expect(point.x).toBeLessThanOrEqual(392 + 1e-6);
    expect(point.y).toBeGreaterThanOrEqual(20 - 1e-6);
    expect(point.y).toBeLessThanOrEqual(30 + 1e-6);
  });
});
