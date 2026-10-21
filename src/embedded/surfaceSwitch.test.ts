import { describe, expect, it } from "vitest";
import { TaskSurface } from "../types";
import { AgentActivity } from "../services/agentActivity";
import { interruptedAgent, landingSurface, planTaskOpen, surfaceLabel } from "./surfaceSwitch";

const vsCode: TaskSurface = { kind: "editor" };
const openCode: TaskSurface = { kind: "opencode" };
const claudeChat: TaskSurface = { kind: "chat", agent: "claude" };
const claudeTerminal: TaskSurface = { kind: "terminal", agent: "claude" };
const codexChat: TaskSurface = { kind: "chat", agent: "codex" };

const working = (activity: Pick<AgentActivity, "agent" | "surface">): AgentActivity => ({
  ...activity,
  state: "working",
  unread: false,
  at: 1,
});

describe("planTaskOpen", () => {
  it("opens the requested surface when nothing is live", () => {
    expect(planTaskOpen([], openCode)).toEqual({ kind: "open", surface: openCode });
  });

  it("opens the requested surface when it is already the live one", () => {
    expect(planTaskOpen([vsCode, claudeChat], claudeChat)).toEqual({
      kind: "open",
      surface: claudeChat,
    });
  });

  it("asks before a different surface replaces the live ones", () => {
    expect(planTaskOpen([vsCode, codexChat], claudeChat)).toEqual({
      kind: "confirm",
      current: vsCode,
      requested: claudeChat,
      closing: [vsCode, codexChat],
    });
  });

  it("tells the chat and terminal of an agent apart", () => {
    expect(planTaskOpen([claudeChat], claudeTerminal).kind).toBe("confirm");
  });

  it("never asks for an external editor", () => {
    expect(planTaskOpen([vsCode], { kind: "external" })).toEqual({
      kind: "open",
      surface: { kind: "external" },
    });
  });
});

describe("landingSurface", () => {
  it("lands on the requested surface when it is live, else on the live one", () => {
    expect(landingSurface([vsCode, claudeChat], claudeChat)).toEqual(claudeChat);
    expect(landingSurface([vsCode], openCode)).toEqual(vsCode);
    expect(landingSurface([vsCode], { kind: "external" })).toEqual(vsCode);
  });

  it("falls back to the requested surface when nothing is live", () => {
    expect(landingSurface([], openCode)).toEqual(openCode);
  });
});

describe("surfaceLabel", () => {
  it("names every surface the dialog can mention", () => {
    expect([vsCode, openCode, claudeChat, claudeTerminal, codexChat].map(surfaceLabel)).toEqual([
      "VSCode",
      "OpenCode",
      "Claude Code chat",
      "Claude Code terminal",
      "Codex chat",
    ]);
    expect(surfaceLabel({ kind: "external" })).toBe("an external editor");
  });
});

describe("interruptedAgent", () => {
  it("names the agent working in a surface that would close", () => {
    expect(interruptedAgent([claudeChat], working({ agent: "claude", surface: "chat" }))).toBe(
      "Claude Code"
    );
    expect(interruptedAgent([openCode], working({ agent: "opencode", surface: "opencode" }))).toBe(
      "OpenCode"
    );
    expect(interruptedAgent([vsCode], working({ agent: "codex", surface: "editor" }))).toBe(
      "Codex"
    );
  });

  it("stays quiet when the agent is idle, elsewhere or absent", () => {
    const waiting = { ...working({ agent: "claude", surface: "chat" }), state: "waiting" as const };
    expect(interruptedAgent([claudeChat], waiting)).toBeNull();
    expect(interruptedAgent([vsCode], working({ agent: "claude", surface: "chat" }))).toBeNull();
    expect(
      interruptedAgent([vsCode], working({ agent: "claude", surface: "external" }))
    ).toBeNull();
    expect(interruptedAgent([vsCode])).toBeNull();
  });
});
