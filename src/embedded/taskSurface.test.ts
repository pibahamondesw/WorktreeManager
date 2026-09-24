import { describe, expect, it } from "vitest";
import { isEmbedded, taskSurfaceFor } from "./taskSurface";

describe("taskSurfaceFor", () => {
  it("opens agents in chat by default and honours a remembered terminal view", () => {
    expect(taskSurfaceFor("claude-code")).toEqual({ kind: "chat", agent: "claude" });
    expect(taskSurfaceFor("codex")).toEqual({ kind: "chat", agent: "codex" });
    expect(isEmbedded(taskSurfaceFor("codex"))).toBe(true);
    const views = { claude: "terminal", codex: "chat" } as const;
    expect(taskSurfaceFor("claude-code", views)).toEqual({ kind: "terminal", agent: "claude" });
    expect(taskSurfaceFor("codex", views)).toEqual({ kind: "chat", agent: "codex" });
  });

  it("keeps every other editor external", () => {
    for (const editor of ["cursor", "vscode", "zed", "opencode"] as const) {
      expect(taskSurfaceFor(editor)).toEqual({ kind: "external" });
      expect(isEmbedded(taskSurfaceFor(editor))).toBe(false);
    }
  });
});
