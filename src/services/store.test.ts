import { beforeEach, describe, expect, it, vi } from "vitest";

const STORE = "store.json";
const SIDECAR = "store.backup-preMultiRepo.json";

// In-memory stand-in for the store plugin. `files` holds the open handles; `savedPaths`
// records which ones were actually flushed, so a test can tell an untouched sidecar from
// one that was created empty.
const { files, savedPaths } = vi.hoisted(() => ({
  files: new Map<string, Map<string, unknown>>(),
  savedPaths: new Set<string>(),
}));

vi.mock("@tauri-apps/plugin-store", () => ({
  load: async (path: string) => {
    if (!files.has(path)) files.set(path, new Map());
    const data = files.get(path)!;
    return {
      get: async (key: string) => data.get(key),
      set: async (key: string, value: unknown) => void data.set(key, value),
      delete: async (key: string) => data.delete(key),
      clear: async () => data.clear(),
      keys: async () => [...data.keys()],
      save: async () => void savedPaths.add(path),
    };
  },
}));

function seed(path: string, contents: Record<string, unknown>): void {
  files.set(path, new Map(Object.entries(contents)));
}

function read(path: string): Record<string, unknown> {
  return Object.fromEntries(files.get(path) ?? new Map());
}

/** Re-imports the module so its cached store handle is rebuilt against the current fixture. */
async function loadState() {
  vi.resetModules();
  return (await import("./store")).loadState();
}

const workspace = {
  id: "w1",
  name: "api",
  linearApiKey: "lin_live",
  repos: [{ id: "r1", name: "api", localPath: "/a", worktreeBasePath: "/wt/a" }],
};

beforeEach(() => {
  files.clear();
  savedPaths.clear();
});

describe("loadState v2 → v3 migration", () => {
  function seedV2(): void {
    seed(STORE, {
      schemaVersion: 2,
      setup: { linearApiKey: "lin_live", isComplete: true, githubToken: "github_pat_dead" },
      vault: { enabled: false, path: null },
      workspaces: [workspace],
      tasks: [],
      selectedWorkspaceId: "w1",
      // pre-multi-repo leftovers the migration is meant to clear
      repos: [{ id: "r1", name: "api", localPath: "/a", linearApiKey: "lin_stale" }],
      worktrees: [{ id: "wt1", repoId: "r1", path: "/wt/a/x" }],
      selectedRepoId: "r1",
    });
    seed(SIDECAR, {
      repos: [{ id: "r1", linearApiKey: "lin_stale" }],
      setup: { linearApiKey: "lin_live", githubToken: "github_pat_dead" },
    });
  }

  it("takes the orphaned GitHub token off disk", async () => {
    seedV2();

    await loadState();

    expect(read(STORE).setup).toEqual({ linearApiKey: "lin_live", isComplete: true });
    expect(JSON.stringify(read(STORE))).not.toContain("github_pat_dead");
  });

  it("keeps the orphaned GitHub token out of the returned state", async () => {
    seedV2();

    const state = await loadState();

    expect(state.setup).toEqual({ linearApiKey: "lin_live", isComplete: true });
  });

  it("deletes the pre-multi-repo keys and their stale Linear copies", async () => {
    seedV2();

    await loadState();

    const stored = read(STORE);
    expect(stored).not.toHaveProperty("repos");
    expect(stored).not.toHaveProperty("worktrees");
    expect(stored).not.toHaveProperty("selectedRepoId");
    expect(JSON.stringify(stored)).not.toContain("lin_stale");
  });

  it("empties the backup sidecar", async () => {
    seedV2();

    await loadState();

    expect(read(SIDECAR)).toEqual({});
  });

  it("preserves the live workspace state", async () => {
    seedV2();

    const state = await loadState();

    expect(state.workspaces).toEqual([workspace]);
    expect(state.selectedWorkspaceId).toBe("w1");
    expect(read(STORE).schemaVersion).toBe(3);
  });

  it("leaves an absent sidecar absent rather than writing an empty one", async () => {
    seedV2();
    files.delete(SIDECAR);

    await loadState();

    expect(savedPaths.has(SIDECAR)).toBe(false);
  });
});

describe("loadState on an already-migrated store", () => {
  it("does not rewrite a v3 store", async () => {
    seed(STORE, {
      schemaVersion: 3,
      setup: { linearApiKey: "lin_live", isComplete: true },
      vault: { enabled: false, path: null },
      workspaces: [workspace],
      tasks: [],
      selectedWorkspaceId: "w1",
    });

    await loadState();

    expect(savedPaths.has(STORE)).toBe(false);
  });
});

describe("loadState v0 → v3 migration", () => {
  function seedV0(): void {
    seed(STORE, {
      setup: { linearApiKey: "lin_live", isComplete: true, githubToken: "github_pat_dead" },
      repos: [{ id: "r1", name: "api", localPath: "/a", worktreeBasePath: "/wt/a" }],
      worktrees: [{ id: "wt1", repoId: "r1", branchName: "feat/x", path: "/wt/a/x" }],
      selectedRepoId: "r1",
    });
  }

  it("lands on v3 with the legacy keys already gone", async () => {
    seedV0();

    await loadState();

    const stored = read(STORE);
    expect(stored.schemaVersion).toBe(3);
    expect(stored).not.toHaveProperty("repos");
    expect(stored).not.toHaveProperty("worktrees");
    expect(stored).not.toHaveProperty("selectedRepoId");
  });

  it("carries the Linear key onto the migrated workspace", async () => {
    seedV0();

    const state = await loadState();

    expect(state.workspaces).toHaveLength(1);
    expect(state.workspaces[0].linearApiKey).toBe("lin_live");
    expect(state.setup).toEqual({ linearApiKey: "lin_live", isComplete: true });
  });

  it("writes a credential-free backup sidecar", async () => {
    seedV0();

    await loadState();

    const sidecar = read(SIDECAR);
    expect(sidecar.repos).toEqual([
      { id: "r1", name: "api", localPath: "/a", worktreeBasePath: "/wt/a" },
    ]);
    expect(sidecar.worktrees).toHaveLength(1);
    expect(JSON.stringify(sidecar)).not.toContain("lin_live");
    expect(JSON.stringify(sidecar)).not.toContain("github_pat_dead");
  });
});
