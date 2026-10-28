import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { InterfacePreferences, Preferences } from "./preferences";

const { data, save, set } = vi.hoisted(() => ({
  data: new Map<string, unknown>(),
  save: vi.fn(),
  set: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-store", () => ({
  load: async () => ({
    get: async (key: string) => data.get(key),
    set: async (key: string, value: unknown) => {
      set(key, value);
      data.set(key, structuredClone(value));
    },
    delete: async (key: string) => data.delete(key),
    save,
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));

const values: Preferences = {
  vault: { enabled: true, path: "/vault" },
  shortcutOverrides: { "future.action": [{ key: "y", modifiers: ["meta"] }] },
  editorApp: "cursor",
  agentViews: { claude: "terminal", codex: "chat" },
  themeId: "ocean",
  customTheme: { accent: "#123456" },
  sidebarCollapsed: true,
  keepRunningInBackground: false,
  scriptApprovals: { "/repo": ["sha256"] },
  agentAlerts: {
    sounds: { done: null, waiting: "Ping" },
    notifications: { done: false, waiting: true },
  },
  navigationHistory: [{ kind: "task", taskId: "t1", workspaceId: "w1", at: "2026-10-28" }],
};

beforeEach(() => {
  vi.resetModules();
  data.clear();
  save.mockReset().mockResolvedValue(undefined);
  set.mockReset();
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => vi.restoreAllMocks());

const keys = Object.keys(values) as (keyof Preferences)[];

describe.each(keys)("%s persistence", (key) => {
  it("round-trips through the store without changing the persisted shape", async () => {
    const preferences = await import("./preferences");
    await preferences.persist(preferences.persistedEntries({ [key]: values[key] }));

    expect(data.get(key)).toEqual(values[key]);
    expect(await preferences.loadPreference(key)).toEqual(values[key]);
    expect(save).toHaveBeenCalledOnce();
  });

  it("restores the previous value on save failure and allows retry", async () => {
    const preferences = await import("./preferences");
    const previous = await preferences.loadPreference(key);
    data.set(key, previous);
    save.mockRejectedValueOnce(new Error("disk full"));

    await expect(
      preferences.persist(preferences.persistedEntries({ [key]: values[key] }))
    ).rejects.toThrow("disk full");
    expect(data.get(key)).toEqual(previous);
    expect(await preferences.loadPreference(key)).toEqual(previous);

    await preferences.persist(preferences.persistedEntries({ [key]: values[key] }));
    expect(await preferences.loadPreference(key)).toEqual(values[key]);
  });

  it("removes a newly introduced key when saving fails", async () => {
    const preferences = await import("./preferences");
    save.mockRejectedValueOnce(new Error("disk full"));

    await expect(
      preferences.persist(preferences.persistedEntries({ [key]: values[key] }))
    ).rejects.toThrow("disk full");
    expect(data.has(key)).toBe(false);
  });
});

describe("normalization", () => {
  it.each([undefined, null, [], 42, "invalid"])(
    "loads safe defaults for malformed values: %s",
    async (raw) => {
      const { loadPreference, normalizers } = await import("./preferences");
      for (const key of keys) {
        data.set(key, raw);
        expect(await loadPreference(key)).toEqual(normalizers[key](undefined));
      }
    }
  );

  it("keeps valid fields and strips malformed or unknown fields", async () => {
    const { normalizers } = await import("./preferences");
    expect(normalizers.vault({ enabled: "true", path: 42, extra: true })).toEqual({
      enabled: false,
      path: null,
    });
    expect(normalizers.vault({ enabled: false, path: "/old-vault" })).toEqual({
      enabled: false,
      path: "/old-vault",
    });
    expect(normalizers.vault({ enabled: true, path: "   " })).toEqual({
      enabled: true,
      path: null,
    });
    expect(normalizers.agentViews({ claude: "terminal", codex: "bad", extra: "terminal" })).toEqual(
      { claude: "terminal", codex: "chat" }
    );
    expect(normalizers.customTheme({ accent: "red", broken: 42 })).toEqual({ accent: "red" });
    expect(normalizers.scriptApprovals({ "/a": ["ok", 1], "/b": "bad" })).toEqual({
      "/a": ["ok"],
      "/b": [],
    });
    expect(normalizers.navigationHistory([null, {}, ...values.navigationHistory])).toEqual(
      values.navigationHistory
    );
  });

  it("keeps running in the background unless explicitly turned off", async () => {
    const { normalizers } = await import("./preferences");
    expect(normalizers.keepRunningInBackground(undefined)).toBe(true);
    expect(normalizers.keepRunningInBackground("false")).toBe(true);
    expect(normalizers.keepRunningInBackground(true)).toBe(true);
    expect(normalizers.keepRunningInBackground(false)).toBe(false);
  });
});

describe("shared writer", () => {
  it("rolls back a partially applied batch before the next queued write", async () => {
    const { persist } = await import("./preferences");
    data.set("themeId", "default");
    set
      .mockImplementationOnce(() => {})
      .mockImplementationOnce(() => {
        throw new Error("set failed");
      });

    const failed = persist([
      ["themeId", "ocean"],
      ["sidebarCollapsed", true],
    ]);
    const next = persist([["editorApp", "cursor"]]);

    await expect(failed).rejects.toThrow("set failed");
    await next;
    expect(Object.fromEntries(data)).toEqual({ themeId: "default", editorApp: "cursor" });
  });
});

describe("optimistic preferences", () => {
  const interfaceKeys: (keyof InterfacePreferences)[] = [
    "editorApp",
    "agentViews",
    "themeId",
    "customTheme",
    "sidebarCollapsed",
    "keepRunningInBackground",
  ];

  it.each(interfaceKeys)("publishes and rolls back %s, reporting the failure", async (key) => {
    const { PreferenceSession, defaultInterfacePreferences } = await import("./preferences");
    const publish = vi.fn();
    const report = vi.fn();
    const session = new PreferenceSession(publish, report);
    let rejectSave!: (error: Error) => void;
    save.mockImplementationOnce(
      () =>
        new Promise((_, reject) => {
          rejectSave = reject;
        })
    );

    const writing = session.update({ [key]: values[key] }, "Could not save");
    await vi.waitFor(() => expect(rejectSave).toBeTypeOf("function"));
    expect(session.value[key]).toEqual(values[key]);
    expect(publish).toHaveBeenLastCalledWith(session.value);
    rejectSave(new Error("disk full"));
    await writing;

    expect(session.value).toEqual(defaultInterfacePreferences());
    expect(publish).toHaveBeenLastCalledWith(defaultInterfacePreferences());
    expect(report).toHaveBeenCalledWith("Could not save");
  });

  it("keeps a later optimistic edit visible when an earlier save fails", async () => {
    const { PreferenceSession } = await import("./preferences");
    const session = new PreferenceSession(vi.fn(), vi.fn());
    let rejectFirst!: (error: Error) => void;
    let finishSecond!: () => void;
    save.mockImplementationOnce(
      () =>
        new Promise((_, reject) => {
          rejectFirst = reject;
        })
    );
    save.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishSecond = resolve;
        })
    );

    const first = session.update({ editorApp: "cursor" }, "failed");
    const second = session.update({ themeId: "ocean" }, "failed");
    expect(session.value).toMatchObject({ editorApp: "cursor", themeId: "ocean" });
    await vi.waitFor(() => expect(rejectFirst).toBeTypeOf("function"));
    rejectFirst(new Error("disk full"));
    await first;
    await vi.waitFor(() => expect(finishSecond).toBeTypeOf("function"));

    expect(session.value).toMatchObject({ editorApp: "vscode-web", themeId: "ocean" });
    finishSecond();
    await second;
    expect(data.get("themeId")).toBe("ocean");
    expect(data.has("editorApp")).toBe(false);
  });

  it("computes consecutive changes from the last successful value after rollback", async () => {
    const { PreferenceSession } = await import("./preferences");
    const session = new PreferenceSession(vi.fn(), vi.fn());
    save.mockRejectedValueOnce(new Error("disk full"));

    const first = session.update({ agentViews: { claude: "terminal", codex: "chat" } }, "failed");
    const second = session.update(
      (current) => ({ agentViews: { ...current.agentViews, codex: "terminal" } }),
      "failed"
    );
    await Promise.all([first, second]);

    expect(session.value.agentViews).toEqual({ claude: "chat", codex: "terminal" });
    expect(data.get("agentViews")).toEqual(session.value.agentViews);
  });
});
