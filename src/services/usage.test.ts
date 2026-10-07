import { describe, expect, it } from "vitest";
import { formatPlanName, formatReset, usageTone } from "./usage";

describe("usage", () => {
  it.each([
    ["self_serve_business_prolite", "Self Serve Business ProLite"],
    ["self_serve_business_usage_based", "Self Serve Business Usage Based"],
    ["plus", "Plus"],
    ["pro", "Pro"],
    ["new_plan", "New Plan"],
  ])("formats plan %s for display", (plan, name) => {
    expect(formatPlanName(plan)).toBe(name);
  });
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
