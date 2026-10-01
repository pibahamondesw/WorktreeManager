import { describe, expect, it } from "vitest";
import { formatReset, usageTone } from "./usage";

describe("usage", () => {
  it("warns from 80% and flags near-exhausted limits", () => {
    expect(usageTone(79)).toBe("normal");
    expect(usageTone(80)).toBe("warning");
    expect(usageTone(95)).toBe("danger");
  });

  it("shows only the time for a reset later today and the day otherwise", () => {
    const now = new Date(2026, 8, 30, 9, 0).getTime();
    const today = new Date(2026, 8, 30, 18, 30).getTime() / 1000;
    const later = new Date(2026, 9, 4, 10, 0).getTime() / 1000;
    expect(formatReset(today, now)).toMatch(/^Resets at /);
    expect(formatReset(later, now)).toMatch(/^Resets \w+, /);
  });
});
