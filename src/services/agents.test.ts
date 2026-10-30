import { beforeEach, describe, expect, it, vi } from "vitest";
import { AGENT_CATALOG } from "./agents.fixture";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

type AgentsModule = typeof import("./agents");
let agents: AgentsModule;

beforeEach(async () => {
  vi.resetModules();
  invoke.mockReset().mockResolvedValue(AGENT_CATALOG);
  agents = await import("./agents");
});

describe("agent catalog", () => {
  it("shares one read between concurrent callers and reads again afterwards", async () => {
    expect(agents.cachedAgentCatalog()).toEqual([]);
    const [first, second] = await Promise.all([
      agents.loadAgentCatalog(),
      agents.loadAgentCatalog(),
    ]);
    expect(first).toBe(second);
    expect(invoke).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenCalledWith("agents_catalog");
    expect(agents.cachedAgentCatalog()).toEqual(AGENT_CATALOG);

    await agents.loadAgentCatalog();
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("keeps the last catalog when a read fails and retries on the next load", async () => {
    await agents.loadAgentCatalog();
    invoke.mockRejectedValueOnce("backend gone").mockResolvedValueOnce(undefined);
    await expect(agents.loadAgentCatalog()).rejects.toBe("backend gone");
    await expect(agents.loadAgentCatalog()).rejects.toThrow("Could not read the agent catalog");
    expect(agents.cachedAgentCatalog()).toEqual(AGENT_CATALOG);
    await agents.loadAgentCatalog();
    expect(invoke).toHaveBeenCalledTimes(4);
  });

  it("describes agents outside the catalog by their id and with no capabilities", async () => {
    expect(agents.describeAgent("claude").label).toBe("claude");
    await agents.loadAgentCatalog();
    expect(agents.describeAgent("claude")).toBe(AGENT_CATALOG[0]);
    const unknown = agents.describeAgent("gemini");
    expect(unknown).toMatchObject({ id: "gemini", label: "gemini", installed: false });
    expect(unknown.capabilities).toMatchObject({
      fork: null,
      headless: false,
      permissionModes: [],
    });
  });

  it("validates ids against the catalog, reading it only when none was read yet", async () => {
    expect(await agents.isCatalogAgent("codex")).toBe(true);
    expect(await agents.isCatalogAgent("gemini")).toBe(false);
    expect(invoke).toHaveBeenCalledOnce();
  });

  it("lists installed chat agents in catalog order and keeps the current one", () => {
    const catalog = AGENT_CATALOG.map((agent) => ({ ...agent, installed: agent.id === "codex" }));
    expect(agents.chatAgents(catalog, "codex")).toEqual(["codex"]);
    expect(agents.chatAgents(catalog, "claude")).toEqual(["claude", "codex"]);
    expect(agents.chatAgents(catalog, "gemini")).toEqual(["codex", "gemini"]);
    const terminalOnly = AGENT_CATALOG.map((agent) => ({
      ...agent,
      capabilities: { ...agent.capabilities, headless: agent.id === "claude" },
    }));
    expect(agents.chatAgents(terminalOnly, "claude")).toEqual(["claude"]);
  });

  it("gives agents without presentation a neutral tone", () => {
    expect(agents.agentPresentation("codex").planUrl).toBeDefined();
    expect(Object.keys(agents.agentPresentation("gemini"))).toEqual(["tone"]);
    expect(agents.agentPresentation("constructor")).toEqual(agents.agentPresentation("gemini"));
    expect(agents.agentPresentation("gemini").tone).not.toEqual(
      agents.agentPresentation("claude").tone
    );
  });
});
