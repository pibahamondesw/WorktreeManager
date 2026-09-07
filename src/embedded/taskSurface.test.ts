import { describe, expect, it } from "vitest";
import { isEmbedded, taskSurfaceFor } from "./taskSurface";

describe("taskSurfaceFor", () => {
  it("embeds a Claude terminal for the claude-code editor", () => {
    expect(taskSurfaceFor("claude-code")).toEqual({ kind: "terminal", agent: "claude" });
    expect(isEmbedded(taskSurfaceFor("claude-code"))).toBe(true);
  });

  it("keeps every other editor external", () => {
    for (const editor of ["cursor", "vscode", "zed", "opencode"] as const) {
      expect(taskSurfaceFor(editor)).toEqual({ kind: "external" });
      expect(isEmbedded(taskSurfaceFor(editor))).toBe(false);
    }
  });
});
