import { AgentDescriptor } from "./agents";

const capabilities: AgentDescriptor["capabilities"] = {
  fork: "relaunch",
  review: false,
  rename: true,
  archive: false,
  models: true,
  effort: true,
  permissionModes: ["interactive", "unattended"],
  interrupt: true,
  steer: false,
  compact: true,
  images: true,
  headless: true,
};

/** The catalog `agents_catalog` returns for the built-in agents, both installed. */
export const AGENT_CATALOG: AgentDescriptor[] = [
  { id: "claude", label: "Claude Code", installed: true, capabilities },
  {
    id: "codex",
    label: "Codex",
    installed: true,
    capabilities: { ...capabilities, fork: "inSession", review: true, archive: true, steer: true },
  },
];

/** Answers `agents_catalog` with `AGENT_CATALOG` and hands every other command to `invoke`. */
export const invokeWithAgentCatalog = <Args>(
  invoke: (command: string, args?: Args) => unknown,
  command: string,
  args?: Args
) => (command === "agents_catalog" ? Promise.resolve(AGENT_CATALOG) : invoke(command, args));
