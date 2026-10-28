import { load, Store } from "@tauri-apps/plugin-store";
import {
  AppState,
  AgentViews,
  DEFAULT_EDITOR_APP,
  EDITOR_APPS,
  EditorApp,
  VaultConfig,
} from "../types";
import {
  EMPTY_SECRETS,
  collectSecrets,
  setupWithoutSecret,
  workspacesWithoutSecrets,
  normalizeSetup,
  normalizeWorkspaces,
  normalizeTasks,
} from "../utils";
import { saveSecrets } from "./keychain";
import { normalizeOverrides, ShortcutOverrides } from "../shortcuts/catalog";
import { NavigationEntry, isNavigationEntry } from "../navigation/history";
import { AgentAlertSettings, normalizeAgentAlerts } from "./agentActivity";
import { SYSTEM_THEME_ID, CUSTOM_THEME_ID, themes } from "../themes";

export interface Preferences {
  vault: VaultConfig;
  shortcutOverrides: ShortcutOverrides;
  editorApp: EditorApp;
  agentViews: AgentViews;
  themeId: string;
  customTheme: Record<string, string> | null;
  sidebarCollapsed: boolean;
  scriptApprovals: Record<string, string[]>;
  agentAlerts: AgentAlertSettings;
  navigationHistory: NavigationEntry[];
}

export type PersistedValues = Omit<AppState, "shortcutOverrides"> &
  Preferences & { schemaVersion: number };
export type PersistedEntry = {
  [K in keyof PersistedValues]: [K, PersistedValues[K]];
}[keyof PersistedValues];

export function persistedEntries(values: Partial<PersistedValues>): PersistedEntry[] {
  return Object.entries(values) as PersistedEntry[];
}

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

const RETIRED_EDITOR_IDS: Record<string, EditorApp> = {
  "cursor-claude": "cursor",
  vscode: "vscode-web",
  "vscode-claude": "vscode-web",
  opencode: "opencode-web",
  "zed-claude": "zed",
  "claude-chat": "claude-code",
  "codex-chat": "codex",
};

export const normalizers: { [K in keyof PersistedValues]: (raw: unknown) => PersistedValues[K] } = {
  setup: normalizeSetup,
  workspaces: normalizeWorkspaces,
  tasks: normalizeTasks,
  selectedWorkspaceId: (raw) => (typeof raw === "string" ? raw : null),
  schemaVersion: (raw) => (typeof raw === "number" && Number.isInteger(raw) && raw >= 0 ? raw : 0),
  vault: (raw) => {
    const value = record(raw);
    return {
      enabled: value.enabled === true,
      path: typeof value.path === "string" && value.path.trim() ? value.path : null,
    };
  },
  shortcutOverrides: normalizeOverrides,
  editorApp: (raw) => {
    const editor = typeof raw === "string" ? (RETIRED_EDITOR_IDS[raw] ?? raw) : null;
    return EDITOR_APPS.some((e) => e.id === editor) ? (editor as EditorApp) : DEFAULT_EDITOR_APP;
  },
  agentViews: (raw) => {
    const value = record(raw);
    return {
      claude: value.claude === "terminal" ? "terminal" : "chat",
      codex: value.codex === "terminal" ? "terminal" : "chat",
    };
  },
  themeId: (raw) =>
    raw === CUSTOM_THEME_ID || raw === SYSTEM_THEME_ID || themes.some((theme) => theme.id === raw)
      ? (raw as string)
      : SYSTEM_THEME_ID,
  customTheme: (raw) =>
    raw === null || typeof raw !== "object" || Array.isArray(raw)
      ? null
      : Object.fromEntries(Object.entries(raw).filter(([, value]) => typeof value === "string")),
  sidebarCollapsed: (raw) => raw === true,
  scriptApprovals: (raw) =>
    Object.fromEntries(
      Object.entries(record(raw)).map(([path, hashes]) => [
        path,
        Array.isArray(hashes)
          ? hashes.filter((hash): hash is string => typeof hash === "string")
          : [],
      ])
    ),
  agentAlerts: normalizeAgentAlerts,
  navigationHistory: (raw) => (Array.isArray(raw) ? raw.filter(isNavigationEntry) : []),
};

export async function loadPreference<K extends keyof PersistedValues>(
  key: K
): Promise<PersistedValues[K]> {
  return normalizers[key](await (await getStore()).get<unknown>(key));
}

export const credentialState = { secrets: EMPTY_SECRETS, readable: true };
let store: Promise<Store> | null = null;
let pendingPersist: Promise<unknown> = Promise.resolve();

export async function getStore(): Promise<Store> {
  if (!store) {
    store = load("store.json", { defaults: {}, autoSave: false }).catch((error) => {
      store = null;
      throw error;
    });
  }
  return store;
}

export function persist(entries: PersistedEntry[]): Promise<void> {
  const result = pendingPersist.then(() => persistEntries(entries));
  pendingPersist = result.catch(() => {});
  return result;
}

async function persistEntries(entries: PersistedEntry[]): Promise<void> {
  try {
    const s = await getStore();
    let next = credentialState.secrets;
    let carriesSecrets = false;
    const toWrite: [string, unknown][] = [];

    for (const [key, value] of entries) {
      if (key === "setup") {
        const setup = value;
        next = { ...next, setup: setup.linearApiKey ?? null };
        carriesSecrets = true;
        toWrite.push([key, setupWithoutSecret(setup)]);
      } else if (key === "workspaces") {
        const workspaces = value;
        next = { ...next, workspaces: collectSecrets(null, workspaces).workspaces };
        carriesSecrets = true;
        toWrite.push([key, workspacesWithoutSecrets(workspaces)]);
      } else {
        toWrite.push([key, value]);
      }
    }

    const previous = await Promise.all(
      toWrite.map(async ([key]) => [key, await s.get(key)] as const)
    );
    const previousSecrets = credentialState.secrets;
    let secretsWritten = false;
    try {
      if (carriesSecrets) {
        if (!credentialState.readable)
          throw new Error("Keychain unreadable; refusing to overwrite the stored secrets");
        await saveSecrets(next);
        secretsWritten = true;
      }
      for (const [key, value] of toWrite) await s.set(key, value);
      await s.save();
      if (secretsWritten) credentialState.secrets = next;
    } catch (error) {
      for (const [key, value] of previous) {
        if (value === undefined) await s.delete(key);
        else await s.set(key, value);
      }
      if (secretsWritten) await saveSecrets(previousSecrets);
      throw error;
    }
  } catch (e) {
    console.error(
      "[persist] failed for keys",
      entries.map(([k]) => k),
      e
    );
    throw e;
  }
}

export type InterfacePreferences = Pick<
  Preferences,
  "editorApp" | "agentViews" | "themeId" | "customTheme" | "sidebarCollapsed"
>;

export function defaultInterfacePreferences(): InterfacePreferences {
  return {
    editorApp: normalizers.editorApp(undefined),
    agentViews: normalizers.agentViews(undefined),
    themeId: normalizers.themeId(undefined),
    customTheme: normalizers.customTheme(undefined),
    sidebarCollapsed: normalizers.sidebarCollapsed(undefined),
  };
}

type PreferenceChange = (current: InterfacePreferences) => Partial<InterfacePreferences>;

export class PreferenceSession {
  private pending: Promise<void> = Promise.resolve();
  private saved: InterfacePreferences = defaultInterfacePreferences();
  private current: InterfacePreferences = this.saved;
  private changes: PreferenceChange[] = [];
  private publish: (value: InterfacePreferences) => void;
  private reportError: (message: string) => void;
  private save: typeof persist;

  constructor(
    publish: (value: InterfacePreferences) => void,
    reportError: (message: string) => void,
    save = persist
  ) {
    this.publish = publish;
    this.reportError = reportError;
    this.save = save;
  }

  get value(): InterfacePreferences {
    return this.current;
  }

  replace(value: InterfacePreferences): void {
    this.saved = value;
    this.refresh();
  }

  private refresh(): void {
    this.current = this.changes.reduce(
      (current, change) => ({ ...current, ...change(current) }),
      this.saved
    );
    this.publish(this.current);
  }

  update(
    change: Partial<InterfacePreferences> | PreferenceChange,
    errorMessage: string
  ): Promise<void> {
    const update: PreferenceChange = typeof change === "function" ? change : () => change;
    this.changes.push(update);
    this.refresh();
    const result = this.pending.then(async () => {
      const patch = update(this.saved);
      try {
        await this.save(persistedEntries(patch));
        this.saved = { ...this.saved, ...patch };
      } catch {
        this.reportError(errorMessage);
      } finally {
        this.changes.shift();
        this.refresh();
      }
    });
    this.pending = result.catch(() => {});
    return result;
  }
}
