import { describe, expect, it } from "vitest";
import { cardScrollDelta } from "./cardScroll";

const container = { top: 100, bottom: 500 };

describe("cardScrollDelta", () => {
  it("leaves a fully visible card in place", () => {
    expect(cardScrollDelta(container, { top: 150, bottom: 300 }, 24)).toBe(0);
  });

  it("aligns a card below the viewport near the container top", () => {
    expect(cardScrollDelta(container, { top: 800, bottom: 900 }, 24)).toBe(676);
  });

  it("aligns a card above the viewport near the container top", () => {
    expect(cardScrollDelta(container, { top: 20, bottom: 120 }, 24)).toBe(-104);
  });

  it("scrolls a card that only overflows the bottom edge", () => {
    expect(cardScrollDelta(container, { top: 450, bottom: 560 }, 24)).toBe(326);
  });
});
