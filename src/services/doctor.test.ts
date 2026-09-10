import { describe, expect, it, vi } from "vitest";
import { DoctorConfig, ProbeReport, runDoctor } from "./doctor";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

function probeReport(overrides: Partial<ProbeReport> = {}): ProbeReport {
  return {
    clis: [],
    apps: [],
    usage: { package_managers: [], doppler: false },
    ...overrides,
  };
}

function found(name: string, version = `${name} 1.0.0`) {
  return { name, path: `/opt/homebrew/bin/${name}`, version };
}

function absent(name: string) {
  return { name, path: null, version: null };
}

function config(overrides: Partial<DoctorConfig> = {}): DoctorConfig {
  return {
    editor: "cursor",
    vaultEnabled: false,
    repoPaths: ["/repos/app"],
    linearKeys: [],
    ...overrides,
  };
}

/** Everything green for the default config, so each test only sets up what it is about. */
function healthyProbe(overrides: Partial<ProbeReport> = {}): ProbeReport {
  return probeReport({
    clis: [found("git", "git version 2.51.0"), found("gh")],
    apps: [{ name: "Cursor", installed: true }],
    ...overrides,
  });
}

function run(cfg: DoctorConfig, probe: ProbeReport, online = true) {
  return runDoctor(cfg, {
    probe: () => Promise.resolve(probe),
    validateKey: () => Promise.resolve({ valid: true, name: "Ada" }),
    online: () => online,
  });
}

function check(report: Awaited<ReturnType<typeof runDoctor>>, id: string) {
  return report.checks.find((c) => c.id === id);
}

describe("runDoctor", () => {
  it("checks the managed embedded runtime instead of accepting any code-server on PATH", async () => {
    const runtime = vi
      .fn()
      .mockResolvedValue({
        installed: true,
        ready: false,
        version: "4.136.2",
        detail: "Repair the editor",
      });
    const report = await runDoctor(config({ editor: "vscode-web" }), {
      probe: async () => healthyProbe({ apps: [] }),
      editorProbe: runtime,
    });
    expect(check(report, "embedded-editor")).toMatchObject({
      severity: "error",
      status: "broken",
      detail: "Repair the editor",
    });
    expect(check(report, "cli:code-server")).toBeUndefined();
    runtime.mockResolvedValue({
      installed: true,
      ready: true,
      version: "4.136.2",
      detail: "Ready",
    });
    const ready = await runDoctor(config({ editor: "vscode-web" }), {
      probe: async () => healthyProbe({ apps: [] }),
      editorProbe: runtime,
    });
    expect(check(ready, "embedded-editor")?.severity).toBe("ok");
  });
  it("reports no problems when everything the setup needs is present", async () => {
    const report = await run(config(), healthyProbe());
    expect(report.errors).toBe(0);
    expect(report.warnings).toBe(0);
    expect(check(report, "cli:git")?.detail).toBe("git version 2.51.0");
  });

  it("probes the CLIs and apps the selected editor needs", async () => {
    const probed = async (editor: DoctorConfig["editor"], vaultEnabled = false) => {
      let input: { clis: string[]; apps: string[]; repoPaths: string[] } | undefined;
      await runDoctor(config({ editor, vaultEnabled }), {
        probe: (received) => {
          input = received;
          return Promise.resolve(healthyProbe());
        },
        validateKey: () => Promise.resolve({ valid: true }),
        online: () => true,
      });
      if (!input) throw new Error("probe was never called");
      return input;
    };

    const zed = await probed("zed", true);
    expect(zed.apps).toEqual(["Zed", "Obsidian"]);
    expect(zed.clis).toContain("zed");
    expect(zed.repoPaths).toEqual(["/repos/app"]);
    expect(new Set(zed.clis).size).toBe(zed.clis.length);

    const claudeCode = await probed("claude-code");
    expect(claudeCode.apps).toEqual([]);
    expect(claudeCode.clis).toContain("claude");
  });

  it("treats a missing editor CLI as an error and a missing optional one as a warning", async () => {
    const required = await run(
      config({ editor: "claude-code" }),
      healthyProbe({ clis: [found("git"), found("gh"), absent("claude")], apps: [] })
    );
    expect(check(required, "cli:claude")?.severity).toBe("error");
    expect(required.errors).toBe(1);

    const optional = await run(
      config({ editor: "zed" }),
      healthyProbe({
        clis: [found("git"), found("gh"), absent("zed")],
        apps: [{ name: "Zed", installed: true }],
      })
    );
    expect(check(optional, "cli:zed")?.severity).toBe("warning");
    expect(optional.errors).toBe(0);
  });

  it("flags a binary that resolves but will not run", async () => {
    const report = await run(
      config(),
      healthyProbe({ clis: [{ name: "git", path: "/usr/bin/git", version: null }, found("gh")] })
    );

    const git = check(report, "cli:git");
    expect(git?.status).toBe("broken");
    expect(git?.severity).toBe("error");
    expect(git?.detail).toContain("/usr/bin/git");
    expect(git?.install).toBe("xcode-select --install");
  });

  it("only asks for a package manager the configured repos actually use", async () => {
    const withoutNode = await run(config(), healthyProbe({ clis: [found("git"), found("gh")] }));
    expect(check(withoutNode, "cli:pnpm")).toBeUndefined();
    expect(check(withoutNode, "cli:node")).toBeUndefined();

    const withPnpm = await run(
      config(),
      healthyProbe({
        clis: [found("git"), found("gh"), absent("node"), absent("pnpm"), absent("yarn")],
        usage: { package_managers: ["pnpm"], doppler: false },
      })
    );
    expect(check(withPnpm, "cli:pnpm")?.severity).toBe("warning");
    expect(check(withPnpm, "cli:pnpm")?.scope).toBe("repository");
    expect(withPnpm.errors).toBe(0);
    expect(withPnpm.warnings).toBe(0);
    expect(check(withPnpm, "cli:node")?.severity).toBe("warning");
    expect(check(withPnpm, "cli:yarn")).toBeUndefined();
  });

  it("only asks for doppler when a repo commits a Doppler config", async () => {
    const unused = await run(
      config(),
      healthyProbe({ clis: [found("git"), found("gh"), absent("doppler")] })
    );
    expect(check(unused, "cli:doppler")).toBeUndefined();

    const used = await run(
      config(),
      healthyProbe({
        clis: [found("git"), found("gh"), absent("doppler")],
        usage: { package_managers: [], doppler: true },
      })
    );
    expect(check(used, "cli:doppler")?.severity).toBe("warning");
    expect(check(used, "cli:doppler")?.scope).toBe("repository");
    expect(used.errors).toBe(0);
    expect(used.warnings).toBe(0);
  });

  it("checks Obsidian only while the vault is enabled", async () => {
    const off = await run(config(), healthyProbe());
    expect(check(off, "app:Obsidian")).toBeUndefined();

    const on = await run(
      config({ vaultEnabled: true }),
      healthyProbe({
        apps: [
          { name: "Cursor", installed: true },
          { name: "Obsidian", installed: false },
        ],
      })
    );
    expect(check(on, "app:Obsidian")?.severity).toBe("error");
  });

  it("orders findings so the fundamentals come first", async () => {
    const report = await run(
      config({ editor: "claude-code" }),
      healthyProbe({
        clis: [found("git"), found("gh"), found("claude")],
        apps: [],
        usage: { package_managers: ["pnpm"], doppler: true },
      })
    );

    expect(report.checks.map((c) => c.id)).toEqual([
      "cli:git",
      "cli:gh",
      "cli:node",
      "cli:pnpm",
      "cli:doppler",
      "cli:claude",
    ]);
  });
});

describe("runDoctor — Linear keys", () => {
  it("reports a Keychain failure instead of missing Linear keys", async () => {
    const validateKey = vi.fn();
    const report = await runDoctor(
      config({
        keychainError: "Failed to read from keychain: authorization denied",
        linearKeys: [{ label: "WorktreeManager", key: null }],
      }),
      { probe: () => Promise.resolve(healthyProbe()), validateKey, online: () => true }
    );
    expect(check(report, "keychain-access")?.detail).toContain("authorization denied");
    expect(check(report, "keychain-access")?.severity).toBe("error");
    expect(check(report, "linear-api-key")).toBeUndefined();
    expect(validateKey).not.toHaveBeenCalled();
  });

  it("skips the check when no workspace exists yet", async () => {
    const report = await run(config(), healthyProbe());
    expect(check(report, "linear-api-key")).toBeUndefined();
  });

  it("names the viewer when the single configured key is valid", async () => {
    const report = await runDoctor(config({ linearKeys: [{ label: "Fintoc", key: "lin_abc" }] }), {
      probe: () => Promise.resolve(healthyProbe()),
      validateKey: () => Promise.resolve({ valid: true, name: "Ada Lovelace" }),
      online: () => true,
    });

    const linear = check(report, "linear-api-key");
    expect(linear?.status).toBe("ok");
    expect(linear?.detail).toBe("Valid — Ada Lovelace");
  });

  it("validates each distinct key once", async () => {
    const validateKey = vi.fn(() => Promise.resolve({ valid: true, name: "Ada" }));
    await runDoctor(
      config({
        linearKeys: [
          { label: "A", key: "shared" },
          { label: "B", key: "shared" },
          { label: "C", key: "other" },
        ],
      }),
      { probe: () => Promise.resolve(healthyProbe()), validateKey, online: () => true }
    );

    expect(validateKey).toHaveBeenCalledTimes(2);
  });

  it("names the workspaces whose key Linear rejected", async () => {
    const report = await runDoctor(
      config({
        linearKeys: [
          { label: "Fintoc", key: "good" },
          { label: "Acme", key: "revoked" },
        ],
      }),
      {
        probe: () => Promise.resolve(healthyProbe()),
        validateKey: (key) =>
          Promise.resolve(
            key === "good"
              ? { valid: true, name: "Ada" }
              : { valid: false, error: "Authentication required" }
          ),
        online: () => true,
      }
    );

    const linear = check(report, "linear-api-key");
    expect(linear?.status).toBe("broken");
    expect(linear?.severity).toBe("error");
    expect(linear?.detail).toBe("Rejected for Acme");
  });

  it("reports a workspace with no key at all", async () => {
    const report = await runDoctor(
      config({
        linearKeys: [
          { label: "Fintoc", key: "good" },
          { label: "Acme", key: null },
          { label: "Blank", key: "   " },
        ],
      }),
      {
        probe: () => Promise.resolve(healthyProbe()),
        validateKey: () => Promise.resolve({ valid: true, name: "Ada" }),
        online: () => true,
      }
    );

    const linear = check(report, "linear-api-key");
    expect(linear?.status).toBe("missing");
    expect(linear?.detail).toBe("No key set for Acme, Blank");
  });

  it("does not call a revoked key when the machine is offline", async () => {
    const validateKey = vi.fn(() => Promise.resolve({ valid: false, error: "nope" }));
    const report = await runDoctor(config({ linearKeys: [{ label: "Fintoc", key: "lin_abc" }] }), {
      probe: () => Promise.resolve(healthyProbe()),
      validateKey,
      online: () => false,
    });

    const linear = check(report, "linear-api-key");
    expect(validateKey).not.toHaveBeenCalled();
    expect(linear?.status).toBe("unknown");
    expect(linear?.severity).toBe("warning");
    expect(linear?.detail).toBe("Could not reach Linear to verify Fintoc");
  });

  it("does not blame the key for a transport failure", async () => {
    const report = await runDoctor(config({ linearKeys: [{ label: "Fintoc", key: "lin_abc" }] }), {
      probe: () => Promise.resolve(healthyProbe()),
      validateKey: () => Promise.resolve({ valid: false, error: "fetch failed: ENOTFOUND" }),
      online: () => true,
    });

    const linear = check(report, "linear-api-key");
    expect(linear?.status).toBe("unknown");
    expect(linear?.severity).toBe("warning");
  });
});
