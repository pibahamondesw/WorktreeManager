import { expect, it } from "vitest";
import {
  MAX_SCALE,
  MIN_SCALE,
  fitView,
  fitWidthView,
  panBy,
  scaleToSlider,
  sliderToScale,
  zoomAt,
} from "./diagramView";

it("fits tall diagrams to the viewport height and centers them", () => {
  expect(fitView({ width: 200, height: 1000 }, { width: 1000, height: 600 }, 50)).toEqual({
    scale: 0.5,
    x: 450,
    y: 50,
  });
});

it("enlarges small diagrams only up to four times their size", () => {
  expect(fitView({ width: 100, height: 50 }, { width: 1000, height: 600 }, 50).scale).toBe(4);
});

it("keeps the anchor point fixed while zooming", () => {
  const view = { x: 100, y: 50, scale: 1 };
  const zoomed = zoomAt(view, 2, { x: 300, y: 250 });

  expect(zoomed).toEqual({ x: -100, y: -150, scale: 2 });
  expect((300 - zoomed.x) / zoomed.scale).toBe((300 - view.x) / view.scale);
});

it("clamps zoom to the supported range", () => {
  const view = { x: 0, y: 0, scale: 1 };

  expect(zoomAt(view, 100, { x: 0, y: 0 }).scale).toBe(MAX_SCALE);
  expect(zoomAt(view, 0, { x: 0, y: 0 }).scale).toBe(MIN_SCALE);
});

it("pans without changing the zoom", () => {
  expect(panBy({ x: 10, y: 20, scale: 3 }, -5, 5)).toEqual({ x: 5, y: 25, scale: 3 });
});

it("fits tall diagrams to the viewport width starting from the top", () => {
  expect(fitWidthView({ width: 200, height: 1000 }, { width: 1000, height: 600 }, 50)).toEqual({
    scale: 4.5,
    x: 50,
    y: 50,
  });
});

it("centers vertically diagrams that fit within the viewport at full width", () => {
  expect(fitWidthView({ width: 900, height: 100 }, { width: 1000, height: 600 }, 50)).toEqual({
    scale: 1,
    x: 50,
    y: 250,
  });
});

it("maps the zoom slider logarithmically so each step grows proportionally", () => {
  expect(sliderToScale(scaleToSlider(1))).toBe(1);
  expect(sliderToScale(scaleToSlider(1) + 1)).toBe(2);
  expect(sliderToScale(100)).toBe(MAX_SCALE);
});
