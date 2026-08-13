import { describe, expect, it } from "vitest";
import { agentSetupLine, defaultVaultPath, taskLogsPath, vaultUri } from "./vault";

describe("defaultVaultPath", () => {
  it("joins under Documents", () => {
    expect(defaultVaultPath("/Users/pedro")).toBe("/Users/pedro/Documents/worktreemanager-vault");
  });

  it("tolerates a trailing slash on home", () => {
    expect(defaultVaultPath("/Users/pedro/")).toBe("/Users/pedro/Documents/worktreemanager-vault");
  });
});

describe("taskLogsPath", () => {
  it("appends task-logs to the vault root", () => {
    expect(taskLogsPath({ enabled: true, path: "/v/vault" })).toBe("/v/vault/task-logs");
  });

  it("strips trailing slashes", () => {
    expect(taskLogsPath({ enabled: true, path: "/v/vault/" })).toBe("/v/vault/task-logs");
  });

  it("returns null when disabled", () => {
    expect(taskLogsPath({ enabled: false, path: "/v/vault" })).toBeNull();
  });

  it("returns null when the path is blank", () => {
    expect(taskLogsPath({ enabled: true, path: "  " })).toBeNull();
    expect(taskLogsPath({ enabled: true, path: null })).toBeNull();
  });
});

describe("vaultUri", () => {
  it("encodes the vault path", () => {
    expect(vaultUri({ enabled: true, path: "/v/my vault" })).toBe(
      "obsidian://open?path=%2Fv%2Fmy%20vault"
    );
  });

  it("returns null without a path", () => {
    expect(vaultUri({ enabled: false, path: null })).toBeNull();
  });
});

describe("agentSetupLine", () => {
  it("points at agent-setup.md in the vault", () => {
    expect(agentSetupLine({ enabled: true, path: "/v/vault" })).toBe("@/v/vault/agent-setup.md");
  });

  it("returns null without a path", () => {
    expect(agentSetupLine({ enabled: false, path: null })).toBeNull();
  });
});
