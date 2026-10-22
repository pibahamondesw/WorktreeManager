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

  it("reopens the agent a task last used, only when an agent is the editor choice", () => {
    expect(taskSurfaceFor("claude-code", undefined, "codex")).toEqual({
      kind: "chat",
      agent: "codex",
    });
    const views = { claude: "chat", codex: "terminal" } as const;
    expect(taskSurfaceFor("claude-code", views, "codex")).toEqual({
      kind: "terminal",
      agent: "codex",
    });
    expect(taskSurfaceFor("cursor", views, "codex")).toEqual({ kind: "external" });
    expect(taskSurfaceFor("vscode-web", views, "codex")).toEqual({ kind: "editor" });
  });

  it("opens the embedded editors inside the app", () => {
    expect(taskSurfaceFor("vscode-web")).toEqual({ kind: "editor" });
    expect(taskSurfaceFor("opencode-web")).toEqual({ kind: "opencode" });
    expect(isEmbedded(taskSurfaceFor("opencode-web"))).toBe(true);
  });

  it("keeps every other editor external", () => {
    for (const editor of ["cursor", "zed"] as const) {
      expect(taskSurfaceFor(editor)).toEqual({ kind: "external" });
      expect(isEmbedded(taskSurfaceFor(editor))).toBe(false);
    }
  });
});
