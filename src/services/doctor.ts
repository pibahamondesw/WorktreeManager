import { invoke } from "@tauri-apps/api/core";
import { EditorApp, EDITOR_REQUIREMENTS } from "../types";
import { validateLinearToken } from "./linear";

/**
 * Startup health check. The `.app` bundle has a minimal PATH, so every tool the app shells out
 * to can be absent without any obvious symptom — a worktree that silently skips its dependency
 * install, an editor that never opens. This composes the Rust probe (`doctor_probe`) with a
 * Linear key check into one list of per-dependency findings, and decides which findings actually
 * matter for the current configuration: `doppler` only counts when a repo commits a Doppler
 * config, `pnpm` only when a repo's lockfile asks for it.
 *
 * Advisory only — nothing here gates the app.
 */

// ---- Rust probe payload ----

export interface CliProbe {
  name: string;
  path: string | null;
  version: string | null;
}

export interface AppProbe {
  name: string;
  installed: boolean;
}

export interface RepoUsage {
  package_managers: string[];
  doppler: boolean;
}

export interface ProbeReport {
  clis: CliProbe[];
  apps: AppProbe[];
  usage: RepoUsage;
}

// ---- Findings ----

export type CheckStatus = "ok" | "missing" | "broken" | "unknown";
export type CheckSeverity = "ok" | "warning" | "error";

export interface DependencyCheck {
  id: string;
  label: string;
  scope: "app" | "repository";
  status: CheckStatus;
  /** `error` blocks something the current setup needs; `warning` degrades or wasn't verifiable. */
  severity: CheckSeverity;
  /** What the app uses it for. */
  reason: string;
  /** Version, path, or which workspaces are affected — whatever the probe learned. */
  detail: string | null;
  /** Shell command that installs it. */
  install?: string;
  url?: string;
}

export interface DoctorReport {
  checks: DependencyCheck[];
  errors: number;
  warnings: number;
}

/** A Linear key in use, labelled by the workspace it belongs to. */
export interface LinearKeySource {
  label: string;
  key: string | null;
}

export interface DoctorConfig {
  editor: EditorApp;
  vaultEnabled: boolean;
  /** Main clones of every configured repo, across all workspaces. */
  repoPaths: string[];
  linearKeys: LinearKeySource[];
  keychainError?: string | null;
}

// ---- Catalogue ----

interface ToolMeta {
  label: string;
  reason: string;
  install?: string;
  url?: string;
}

/**
 * Package managers are probed unconditionally — they all live in one shell loop, so probing the
 * four costs the same as probing one, and which of them matters is only known once the repo scan
 * comes back in the same response.
 */
const BASE_CLIS = ["git", "gh", "node", "npm", "pnpm", "yarn", "bun", "doppler"];

/** Display order; anything unlisted sorts last. */
const CLI_ORDER = ["git", "gh", "node", "npm", "pnpm", "yarn", "bun", "doppler", "claude", "zed"];

const CLI_TOOLS: Record<string, ToolMeta> = {
  git: {
    label: "git",
    reason: "Every worktree operation: add, remove, list, status, fetch.",
    install: "xcode-select --install",
    url: "https://git-scm.com/downloads/mac",
  },
  gh: {
    label: "GitHub CLI (gh)",
    reason:
      "Optional. Installs the git credential helper that lets fetches from a GitHub HTTPS remote authenticate.",
    install: "brew install gh",
    url: "https://cli.github.com",
  },
  node: {
    label: "Node.js",
    reason: "Runs the package manager that installs dependencies in a new worktree.",
    install: "brew install node",
    url: "https://nodejs.org",
  },
  npm: {
    label: "npm",
    reason: "Installs dependencies in a new worktree of a repo with no other lockfile.",
    install: "brew install node",
  },
  pnpm: {
    label: "pnpm",
    reason: "Installs dependencies in a new worktree of a repo with a pnpm lockfile.",
    install: "brew install pnpm",
    url: "https://pnpm.io/installation",
  },
  yarn: {
    label: "Yarn",
    reason: "Installs dependencies in a new worktree of a repo with a Yarn lockfile.",
    install: "brew install yarn",
  },
  bun: {
    label: "Bun",
    reason: "Installs dependencies in a new worktree of a repo with a Bun lockfile.",
    install: "brew install oven-sh/bun/bun",
  },
  doppler: {
    label: "Doppler CLI",
    reason: "Scopes each new worktree to the project its repo's doppler.yaml declares.",
    install: "brew install dopplerhq/cli/doppler",
    url: "https://docs.doppler.com/docs/install-cli",
  },
  claude: {
    label: "Claude Code",
    reason: "Run inside the worktree by the Claude Code editor option.",
    install: "curl -fsSL https://claude.ai/install.sh | bash",
    url: "https://docs.claude.com/en/docs/claude-code/setup",
  },
  zed: {
    label: "Zed CLI",
    reason:
      "Optional. Opens every repo of a workspace in one Zed window; without it only the first opens.",
    install: "brew install --cask zed",
    url: "https://zed.dev/docs/getting-started",
  },
};

const APP_TOOLS: Record<string, ToolMeta> = {
  Cursor: {
    label: "Cursor",
    reason: "Your selected editor.",
    url: "https://cursor.com",
  },
  "Visual Studio Code": {
    label: "Visual Studio Code",
    reason: "Your selected editor.",
    install: "brew install --cask visual-studio-code",
    url: "https://code.visualstudio.com",
  },
  Zed: {
    label: "Zed",
    reason: "Your selected editor.",
    install: "brew install --cask zed",
    url: "https://zed.dev",
  },
  OpenCode: {
    label: "OpenCode",
    reason: "Your selected editor.",
    url: "https://opencode.ai",
  },
  Obsidian: {
    label: "Obsidian",
    reason: "Opens the vault and the task note the app writes for each worktree.",
    install: "brew install --cask obsidian",
    url: "https://obsidian.md",
  },
};

const LINEAR_CHECK_ID = "linear-api-key";
const LINEAR_REASON =
  "Lists your assigned issues and moves one to started when its worktree is created.";

/**
 * A Linear request that never reached Linear must not read as a revoked key, so failures are
 * split on the error text. The SDK surfaces transport failures through `fetch`, which is why
 * that word is in here.
 */
const NETWORK_ERROR =
  /network|fetch|socket|dns|enotfound|econnrefused|econnreset|etimedout|timeout|offline/i;

// ---- Composition ----

interface DoctorDeps {
  probe: (input: { clis: string[]; apps: string[]; repoPaths: string[] }) => Promise<ProbeReport>;
  validateKey: typeof validateLinearToken;
  online: () => boolean;
}

const defaultDeps: DoctorDeps = {
  probe: (input) => invoke<ProbeReport>("doctor_probe", input),
  validateKey: validateLinearToken,
  online: () => navigator.onLine,
};

export async function runDoctor(
  config: DoctorConfig,
  overrides: Partial<DoctorDeps> = {}
): Promise<DoctorReport> {
  const deps = { ...defaultDeps, ...overrides };
  const requirements = EDITOR_REQUIREMENTS[config.editor];

  const apps = [...requirements.apps, ...(config.vaultEnabled ? ["Obsidian"] : [])];
  const clis = unique([...BASE_CLIS, ...requirements.clis, ...requirements.optionalClis]);

  const [probe, linear] = await Promise.all([
    deps.probe({ clis, apps, repoPaths: config.repoPaths }),
    config.keychainError
      ? Promise.resolve<DependencyCheck>({
          id: "keychain-access",
          label: "Keychain access",
          scope: "app",
          status: "broken",
          severity: "error",
          reason:
            "Could not read your saved Linear credentials. Re-check to retry Keychain access.",
          detail: config.keychainError,
        })
      : checkLinearKeys(config.linearKeys, deps),
  ]);

  const checks = [
    ...cliChecks(probe, requirements),
    ...appChecks(probe),
    ...(linear ? [linear] : []),
  ];

  return {
    checks,
    errors: checks.filter((c) => c.scope === "app" && c.severity === "error").length,
    warnings: checks.filter((c) => c.scope === "app" && c.severity === "warning").length,
  };
}

/** Which probed CLIs are worth showing, and whether their absence is an error or a note. */
function cliChecks(
  probe: ProbeReport,
  requirements: (typeof EDITOR_REQUIREMENTS)[EditorApp]
): DependencyCheck[] {
  const { package_managers: managers, doppler } = probe.usage;

  const required = new Set(["git", ...requirements.clis]);
  const repository = new Set(managers);
  if (managers.length > 0) repository.add("node");
  if (doppler) repository.add("doppler");

  const optional = new Set(["gh", ...requirements.optionalClis]);

  const byName = new Map(probe.clis.map((c) => [c.name, c]));
  return unique([...required, ...optional, ...repository])
    .sort(byCliOrder)
    .map((name) => ({
      ...cliCheck(name, byName.get(name), required.has(name)),
      scope: repository.has(name) ? ("repository" as const) : ("app" as const),
    }));
}

function byCliOrder(a: string, b: string): number {
  const rank = (n: string) => {
    const i = CLI_ORDER.indexOf(n);
    return i === -1 ? CLI_ORDER.length : i;
  };
  return rank(a) - rank(b) || a.localeCompare(b);
}

function cliCheck(name: string, probe: CliProbe | undefined, required: boolean): DependencyCheck {
  const meta = CLI_TOOLS[name] ?? { label: name, reason: "Used by the app." };
  const status = cliStatus(probe);
  return {
    id: `cli:${name}`,
    scope: "app",
    label: meta.label,
    status,
    severity: severityFor(status, required),
    reason: meta.reason,
    detail: cliDetail(status, probe),
    install: meta.install,
    url: meta.url,
  };
}

function cliStatus(probe: CliProbe | undefined): CheckStatus {
  if (!probe) return "unknown";
  if (!probe.path) return "missing";
  // Resolves but won't run: /usr/bin/git without Xcode Command Line Tools behaves this way.
  if (!probe.version) return "broken";
  return "ok";
}

function cliDetail(status: CheckStatus, probe: CliProbe | undefined): string | null {
  if (status === "ok") return probe?.version ?? null;
  if (status === "broken") return `Found at ${probe?.path} but it fails to run`;
  return null;
}

function appChecks(probe: ProbeReport): DependencyCheck[] {
  return probe.apps.map((app) => {
    const meta = APP_TOOLS[app.name] ?? { label: app.name, reason: "Used by the app." };
    return {
      id: `app:${app.name}`,
      scope: "app" as const,
      label: meta.label,
      status: app.installed ? ("ok" as const) : ("missing" as const),
      severity: app.installed ? ("ok" as const) : ("error" as const),
      reason: meta.reason,
      detail: app.installed ? "Installed" : null,
      install: meta.install,
      url: meta.url,
    };
  });
}

function severityFor(status: CheckStatus, required: boolean): CheckSeverity {
  if (status === "ok") return "ok";
  if (status === "unknown") return "warning";
  return required ? "error" : "warning";
}

/**
 * One finding for every Linear key in play. Each distinct key is validated once, so workspaces
 * sharing a key cost a single request.
 */
async function checkLinearKeys(
  sources: LinearKeySource[],
  deps: Pick<DoctorDeps, "validateKey" | "online">
): Promise<DependencyCheck | null> {
  if (sources.length === 0) return null;

  const base = {
    id: LINEAR_CHECK_ID,
    scope: "app" as const,
    label: "Linear API key",
    reason: LINEAR_REASON,
    url: "https://linear.app/settings/api",
  };

  const unset = sources.filter((s) => !s.key?.trim()).map((s) => s.label);
  const configured = sources.filter((s) => s.key?.trim());

  const rejected: string[] = [];
  const unverified: string[] = [];
  let viewer: string | null = null;

  if (configured.length > 0 && !deps.online()) {
    unverified.push(...configured.map((s) => s.label));
  } else if (configured.length > 0) {
    const results = await validateDistinct(configured, deps.validateKey);
    for (const source of configured) {
      const result = results.get(source.key!.trim())!;
      if (result.valid) viewer = result.name ?? viewer;
      else if (NETWORK_ERROR.test(result.error ?? "")) unverified.push(source.label);
      else rejected.push(source.label);
    }
  }

  const status: CheckStatus = rejected.length
    ? "broken"
    : unset.length
      ? "missing"
      : unverified.length
        ? "unknown"
        : "ok";

  const notes = [
    rejected.length ? `rejected for ${rejected.join(", ")}` : null,
    unset.length ? `no key set for ${unset.join(", ")}` : null,
    unverified.length ? `could not reach Linear to verify ${unverified.join(", ")}` : null,
  ].filter((n): n is string => n !== null);

  return {
    ...base,
    status,
    severity: severityFor(status, true),
    detail: notes.length ? capitalize(notes.join("; ")) : okLinearDetail(configured.length, viewer),
  };
}

async function validateDistinct(
  sources: LinearKeySource[],
  validateKey: DoctorDeps["validateKey"]
): Promise<Map<string, Awaited<ReturnType<DoctorDeps["validateKey"]>>>> {
  const keys = unique(sources.map((s) => s.key!.trim()));
  const results = await Promise.all(
    keys.map(async (key) => [key, await validateKey(key)] as const)
  );
  return new Map(results);
}

function okLinearDetail(count: number, viewer: string | null): string {
  if (count === 1) return viewer ? `Valid — ${viewer}` : "Valid";
  return `Valid for ${count} workspaces`;
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function unique(values: string[]): string[] {
  return [...new Set(values)];
}
