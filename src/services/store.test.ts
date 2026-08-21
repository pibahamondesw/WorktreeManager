import { beforeEach, describe, expect, it, vi } from "vitest";

const STORE = "store.json";
const SIDECAR = "store.backup-preMultiRepo.json";

// In-memory stands-in for the two things loadState talks to. `files` holds the open store
// handles and `savedPaths` records which were actually flushed, so a test can tell an
// untouched sidecar from one created empty. `keychain` doubles as a fault injector: the
// readable/writable flags simulate a denied authorization prompt.
const { files, savedPaths, keychain } = vi.hoisted(() => ({
  files: new Map<string, Map<string, unknown>>(),
  savedPaths: new Set<string>(),
  keychain: { value: null as string | null, readable: true, writable: true },
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

vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "keychain_get") {
      if (!keychain.readable) throw new Error("authorization denied");
      return keychain.value;
    }
    if (cmd === "keychain_set") {
      if (!keychain.writable) throw new Error("authorization denied");
      keychain.value = args!.value as string;
      return undefined;
    }
    throw new Error(`unexpected command: ${cmd}`);
  },
}));

function seed(path: string, contents: Record<string, unknown>): void {
  files.set(path, new Map(Object.entries(contents)));
}

function read(path: string): Record<string, unknown> {
  return Object.fromEntries(files.get(path) ?? new Map());
}

function storedSecrets(): { setup: string | null; workspaces: Record<string, string> } {
  return JSON.parse(keychain.value!);
}

/** Re-imports the module so its cached store handle and secrets are rebuilt per test. */
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
  keychain.value = null;
  keychain.readable = true;
  keychain.writable = true;
});

describe("loadState v2 → v4 migration", () => {
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

  it("leaves an absent sidecar absent rather than writing an empty one", async () => {
    seedV2();
    files.delete(SIDECAR);

    await loadState();

    expect(savedPaths.has(SIDECAR)).toBe(false);
  });

  it("moves every Linear key into the keychain and none is left in the file", async () => {
    seedV2();

    await loadState();

    expect(storedSecrets()).toEqual({ setup: "lin_live", workspaces: { w1: "lin_live" } });
    expect(JSON.stringify(read(STORE))).not.toContain("lin_live");
    expect(read(STORE).schemaVersion).toBe(4);
  });

  it("still hands the keys back to the app in memory", async () => {
    seedV2();

    const state = await loadState();

    expect(state.setup.linearApiKey).toBe("lin_live");
    expect(state.workspaces).toEqual([workspace]);
    expect(state.selectedWorkspaceId).toBe("w1");
  });
});

describe("loadState when the keychain refuses", () => {
  function seedV3(): void {
    seed(STORE, {
      schemaVersion: 3,
      setup: { linearApiKey: "lin_live", isComplete: true },
      vault: { enabled: false, path: null },
      workspaces: [workspace],
      tasks: [],
      selectedWorkspaceId: "w1",
    });
  }

  it("leaves the keys in the file rather than stranding them", async () => {
    seedV3();
    keychain.writable = false;

    await loadState();

    const stored = read(STORE);
    expect(JSON.stringify(stored)).toContain("lin_live");
    expect(stored.schemaVersion).toBe(3);
  });

  it("keeps the app usable while the move is deferred", async () => {
    seedV3();
    keychain.writable = false;

    const state = await loadState();

    expect(state.setup.linearApiKey).toBe("lin_live");
    expect(state.workspaces[0].linearApiKey).toBe("lin_live");
  });

  it("does not strip the file when the write cannot be verified", async () => {
    seedV3();
    keychain.readable = false; // write lands, read-back fails

    await loadState();

    expect(JSON.stringify(read(STORE))).toContain("lin_live");
    expect(read(STORE).schemaVersion).toBe(3);
  });
});

describe("loadState on an already-migrated store", () => {
  function seedV4(): void {
    seed(STORE, {
      schemaVersion: 4,
      setup: { linearApiKey: null, isComplete: true },
      vault: { enabled: false, path: null },
      workspaces: [{ ...workspace, linearApiKey: null }],
      tasks: [],
      selectedWorkspaceId: "w1",
    });
    keychain.value = JSON.stringify({ setup: "lin_live", workspaces: { w1: "lin_ws" } });
  }

  it("does not rewrite the store", async () => {
    seedV4();

    await loadState();

    expect(savedPaths.has(STORE)).toBe(false);
  });

  it("restores the keys from the keychain", async () => {
    seedV4();

    const state = await loadState();

    expect(state.setup.linearApiKey).toBe("lin_live");
    expect(state.workspaces[0].linearApiKey).toBe("lin_ws");
  });

  it("comes up without keys, not broken, when the keychain cannot be read", async () => {
    seedV4();
    keychain.readable = false;

    const state = await loadState();

    expect(state.setup.linearApiKey).toBeNull();
    expect(state.workspaces[0].linearApiKey).toBeNull();
    expect(state.workspaces[0].name).toBe("api");
    expect(savedPaths.has(STORE)).toBe(false);
  });
});

describe("loadState v0 → v4 migration", () => {
  function seedV0(): void {
    seed(STORE, {
      setup: { linearApiKey: "lin_live", isComplete: true, githubToken: "github_pat_dead" },
      repos: [{ id: "r1", name: "api", localPath: "/a", worktreeBasePath: "/wt/a" }],
      worktrees: [{ id: "wt1", repoId: "r1", branchName: "feat/x", path: "/wt/a/x" }],
      selectedRepoId: "r1",
    });
  }

  it("lands on v4 with the legacy keys already gone", async () => {
    seedV0();

    await loadState();

    const stored = read(STORE);
    expect(stored.schemaVersion).toBe(4);
    expect(stored).not.toHaveProperty("repos");
    expect(stored).not.toHaveProperty("worktrees");
    expect(stored).not.toHaveProperty("selectedRepoId");
  });

  it("carries the Linear key onto the migrated workspace and into the keychain", async () => {
    seedV0();

    const state = await loadState();

    expect(state.workspaces).toHaveLength(1);
    expect(state.workspaces[0].linearApiKey).toBe("lin_live");
    expect(state.setup).toEqual({ linearApiKey: "lin_live", isComplete: true });
    expect(storedSecrets().workspaces).toEqual({ r1: "lin_live" });
    expect(JSON.stringify(read(STORE))).not.toContain("lin_live");
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
