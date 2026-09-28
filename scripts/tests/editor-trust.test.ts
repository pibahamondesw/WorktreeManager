import { readFileSync } from "node:fs";
import { webcrypto } from "node:crypto";
import { createContext, runInContext } from "node:vm";
import { IDBFactory, IDBObjectStore } from "fake-indexeddb";
import { afterEach, describe, expect, it, vi } from "vitest";

const source = readFileSync(
  new URL("../../src-tauri/src/commands/code_server/trust.js", import.meta.url),
  "utf8"
);
const databaseName = "vscode-web-state-db-global-shared";
const trustKey = "content.trust.model.key";
const originalPut = IDBObjectStore.prototype.put;
const originalDelete = IDBObjectStore.prototype.delete;
const workspace = "/editor/sessions/new/task.code-workspace";
const origin = "http://127.0.0.1:53172";
const authority = "127.0.0.1:53172";

afterEach(() => {
  IDBObjectStore.prototype.put = originalPut;
  IDBObjectStore.prototype.delete = originalDelete;
});

type TrustChange =
  | { operation: "initialize"; paths: string[] }
  | { operation: "update"; added: string[]; removed: string[] };

interface TrustRequest {
  id: number;
  documentId: string;
  change: TrustChange;
}

interface TrustResponse {
  id?: number;
  documentId?: string;
  snapshot?: { revision: number; paths: string[]; workspaces?: WorkspaceTrust[] };
  error?: string;
}

function trustData(paths: string[], host = authority) {
  return {
    uriTrustInfo: paths.map((path) => ({
      uri: { scheme: "vscode-remote", authority: host, path },
      trusted: true,
    })),
  };
}

function value(paths: string[], host = authority) {
  return JSON.stringify(trustData(paths, host));
}

function openDatabase(factory: IDBFactory, name = databaseName) {
  return new Promise<{ database: IDBDatabase; eventType: unknown }>((resolve, reject) => {
    const request = factory.open(name);
    request.onupgradeneeded = () => request.result.createObjectStore("ItemTable");
    request.onerror = () => reject(request.error);
    request.onsuccess = (event) =>
      resolve({ database: request.result, eventType: event.constructor });
  });
}

function write(database: IDBDatabase, entries: Record<string, string>, abort = false) {
  return new Promise<void>((resolve, reject) => {
    const transaction = database.transaction("ItemTable", "readwrite");
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => {
      if (!abort) reject(transaction.error);
    };
    transaction.onabort = () => (abort ? resolve() : reject(transaction.error));
    for (const [key, content] of Object.entries(entries)) {
      transaction.objectStore("ItemTable").put(content, key);
    }
    if (abort) transaction.abort();
  });
}

function read(database: IDBDatabase, key = trustKey) {
  return new Promise<string>((resolve, reject) => {
    const transaction = database.transaction("ItemTable", "readonly");
    const request = transaction.objectStore("ItemTable").get(key);
    transaction.oncomplete = () => {
      const result: unknown = request.result;
      if (typeof result === "string") resolve(result);
      else reject(new Error("Expected a stored string"));
    };
    transaction.onerror = () => reject(transaction.error);
  });
}

interface WorkspaceTrust {
  workspace: string;
  folders: string[];
}

async function setup(
  options: {
    shared?: string[];
    existing?: string;
    folders?: string[];
    factory?: IDBFactory;
    workspace?: string;
    workspaces?: WorkspaceTrust[];
  } = {}
) {
  const factory = options.factory ?? new IDBFactory();
  const currentWorkspace = options.workspace ?? workspace;
  const folders = options.folders ?? ["/repos/project/task"];
  const workspaces = options.workspaces ?? [{ workspace: currentWorkspace, folders }];
  const seed = await openDatabase(factory);
  await write(seed.database, {
    ...(options.existing ? { [trustKey]: options.existing } : {}),
    "unrelated-state": "keep this",
  });
  seed.database.close();
  const changes: TrustChange[] = [];
  const messages: { changed: Map<string, string> }[] = [];
  const shared = new Set(options.shared ?? []);
  let revision = 0;
  let failure: string | undefined;
  let replyDelay = 0;
  const receive: (response: TrustResponse) => void = () => {
    throw new Error("Trust receiver not installed");
  };
  const sandbox = {
    window: undefined as unknown,
    top: undefined as unknown,
    __worktreeTrustReceive: receive,
    indexedDB: factory,
    IDBObjectStore,
    Event: seed.eventType,
    URL,
    crypto: webcrypto,
    setTimeout,
    clearTimeout,
    console: { error: vi.fn() },
    alert: vi.fn(),
    BroadcastChannel: class {
      postMessage(message: { changed: Map<string, string> }) {
        messages.push(message);
      }
      close() {}
    },
    location: {
      origin,
      host: authority,
      set href(target: string) {
        const url = new URL(target);
        expect(url.pathname).toBe("/__worktreemanager_trust");
        expect(url.searchParams.get("token")).toBe("session-token");
        const request = JSON.parse(url.searchParams.get("message")!) as TrustRequest;
        changes.push(request.change);
        if (request.change.operation === "initialize") {
          for (const path of request.change.paths) shared.add(path);
        } else {
          for (const path of request.change.removed) shared.delete(path);
          for (const path of request.change.added) shared.add(path);
        }
        const response = failure
          ? { id: request.id, documentId: request.documentId, error: failure }
          : {
              id: request.id,
              documentId: request.documentId,
              snapshot: { revision: ++revision, paths: [...shared], workspaces },
            };
        setTimeout(() => context.__worktreeTrustReceive(response), replyDelay);
      },
    },
  };
  const context = createContext(sandbox) as typeof sandbox;
  context.window = context;
  context.top = context;
  runInContext(
    `(${source})(${JSON.stringify({
      origin,
      endpoint: "/__worktreemanager_trust",
      token: "session-token",
      workspace: currentWorkspace,
      folders,
    })})`,
    context
  );
  const { database } = await openDatabase(factory);
  return {
    database,
    factory,
    changes,
    messages,
    shared,
    context,
    delayReplies: (milliseconds: number) => {
      replyDelay = milliseconds;
    },
    fail: (message: string) => {
      failure = message;
    },
    receive: (paths: string[], nextRevision = ++revision) => {
      context.__worktreeTrustReceive({ snapshot: { revision: nextRevision, paths, workspaces } });
    },
  };
}

describe("embedded editor workspace trust", () => {
  it("seeds a new task before VS Code reads storage and covers its generated workspace", async () => {
    const editor = await setup({ shared: ["/repos"] });
    expect(JSON.parse(await read(editor.database))).toEqual(
      JSON.parse(value(["/repos", workspace]))
    );
    expect(await read(editor.database, "unrelated-state")).toBe("keep this");
    expect(editor.context.alert).not.toHaveBeenCalled();
  });

  it("keeps both tasks trusted when their editors write to the same database", async () => {
    const secondWorkspace = "/editor/sessions/second/task.code-workspace";
    const workspaces = [
      { workspace, folders: ["/repos/one"] },
      { workspace: secondWorkspace, folders: ["/repos/two"] },
    ];
    const first = await setup({ shared: ["/repos"], workspaces });
    const second = await setup({
      factory: first.factory,
      shared: ["/repos"],
      workspace: secondWorkspace,
      folders: ["/repos/two"],
      workspaces,
    });
    expect(second.changes[0]).toEqual({ operation: "initialize", paths: ["/repos"] });
    expect(JSON.parse(await read(second.database))).toEqual(
      JSON.parse(value(["/repos", workspace, secondWorkspace]))
    );
    first.receive(["/repos"], 10);
    await vi.waitFor(() =>
      expect(first.messages.at(-1)?.changed.get(trustKey)).toBe(
        value(["/repos", workspace, secondWorkspace])
      )
    );
    expect(await read(second.database)).toBe(value(["/repos", workspace, secondWorkspace]));
    second.receive([], 11);
    await vi.waitFor(async () => expect(await read(first.database)).toBe(value([])));
  });

  it("derives trust for new tasks only when all their repositories are already covered", async () => {
    const secondWorkspace = "/editor/sessions/second/task.code-workspace";
    const editor = await setup({
      shared: ["/repos"],
      workspaces: [
        { workspace, folders: ["/repos/one"] },
        { workspace: secondWorkspace, folders: ["/repos/two", "/untrusted/three"] },
      ],
    });
    expect(await read(editor.database)).toBe(value(["/repos", workspace]));
    editor.receive(["/repos", "/untrusted"], 10);
    await vi.waitFor(async () =>
      expect(await read(editor.database)).toBe(
        value(["/repos", "/untrusted", workspace, secondWorkspace])
      )
    );
    editor.receive(["/repos"], 11);
    await vi.waitFor(async () =>
      expect(await read(editor.database)).toBe(value(["/repos", workspace]))
    );
  });

  it("requires every repository and respects parent path boundaries", async () => {
    const editor = await setup({ shared: ["/repos"], folders: ["/repos/one", "/repos-other/two"] });
    expect(JSON.parse(await read(editor.database))).toEqual(JSON.parse(value(["/repos"])));
    editor.receive(["/repos", "/repos-other"]);
    await vi.waitFor(async () => {
      expect(JSON.parse(await read(editor.database))).toEqual(
        JSON.parse(value(["/repos", "/repos-other", workspace]))
      );
    });
    editor.receive(["/repos"]);
    await vi.waitFor(async () =>
      expect(JSON.parse(await read(editor.database))).toEqual(JSON.parse(value(["/repos"])))
    );
  });

  it("imports local decisions but excludes other remote authorities and generated workspace paths", async () => {
    const existing = JSON.stringify({
      uriTrustInfo: [
        ...trustData(["/legacy", workspace]).uriTrustInfo,
        ...trustData(["/remote"], "ssh-other-machine").uriTrustInfo,
      ],
    });
    const editor = await setup({ shared: ["/repos"], existing });
    expect(editor.changes[0]).toEqual({ operation: "initialize", paths: ["/legacy"] });
    expect(editor.shared).toEqual(new Set(["/repos", "/legacy"]));
    expect(
      (JSON.parse(await read(editor.database)) as ReturnType<typeof trustData>).uriTrustInfo
    ).toContainEqual(trustData(["/remote"], "ssh-other-machine").uriTrustInfo[0]);
  });

  it("publishes committed additions and removals without sharing unrelated state", async () => {
    const editor = await setup({ shared: ["/repos"] });
    await write(editor.database, { [trustKey]: value(["/other"]), "extension-state": "private" });
    await vi.waitFor(() =>
      expect(editor.changes[1]).toEqual({
        operation: "update",
        added: ["/other"],
        removed: ["/repos"],
      })
    );
    await vi.waitFor(async () =>
      expect(JSON.parse(await read(editor.database))).toEqual(JSON.parse(value(["/other"])))
    );
    expect(editor.shared).toEqual(new Set(["/other"]));
    expect(await read(editor.database, "extension-state")).toBe("private");
  });

  it("ignores aborted transactions and other databases", async () => {
    const editor = await setup({ shared: ["/repos"] });
    await write(editor.database, { [trustKey]: value(["/aborted"]) }, true);
    const other = await openDatabase(editor.factory, "workspace-state");
    await write(other.database, { [trustKey]: value(["/unrelated"]) });
    expect(editor.changes).toHaveLength(1);
    expect(await read(editor.database)).toBe(value(["/repos", workspace]));
  });

  it("keeps a rapid grant followed by revocation while the first save is still pending", async () => {
    const editor = await setup();
    editor.delayReplies(30);
    await write(editor.database, { [trustKey]: value(["/repos"]) });
    await write(editor.database, { [trustKey]: value([]) });
    await vi.waitFor(() => expect(editor.changes).toHaveLength(3));
    await vi.waitFor(async () => expect(await read(editor.database)).toBe(value([])));
    expect(editor.shared).toEqual(new Set());
    expect(editor.changes[2]).toEqual({ operation: "update", added: [], removed: ["/repos"] });
  });

  it("shares deletion of the trust key", async () => {
    const editor = await setup({ shared: ["/repos"] });
    const transaction = editor.database.transaction("ItemTable", "readwrite");
    transaction.objectStore("ItemTable").delete(trustKey);
    await vi.waitFor(() =>
      expect(editor.changes[1]).toEqual({ operation: "update", added: [], removed: ["/repos"] })
    );
    await vi.waitFor(async () => expect(await read(editor.database)).toBe(value([])));
  });

  it("ignores a delayed reply from the document before a reload", async () => {
    const editor = await setup();
    editor.delayReplies(30);
    await write(editor.database, { [trustKey]: value(["/repos"]) });
    await vi.waitFor(() => expect(editor.changes).toHaveLength(2));
    editor.context.__worktreeTrustReceive({
      id: 2,
      documentId: "previous-document",
      snapshot: { revision: 1000, paths: ["/stale"] },
    });
    await vi.waitFor(async () =>
      expect(await read(editor.database)).toBe(value(["/repos", workspace]))
    );
    expect(editor.context.alert).not.toHaveBeenCalled();
  });

  it("applies external revocations and ignores older snapshots without a feedback loop", async () => {
    const editor = await setup({ shared: ["/repos"] });
    editor.receive([], 10);
    await vi.waitFor(async () => expect(await read(editor.database)).toBe(value([])));
    editor.receive(["/repos"], 9);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(await read(editor.database)).toBe(value([]));
    expect(editor.changes).toHaveLength(1);
    expect(editor.messages.at(-1)?.changed.get(trustKey)).toBe(value([]));
  });

  it("reports persistence failures instead of silently claiming the decision was shared", async () => {
    const editor = await setup();
    editor.fail("Permission denied");
    await write(editor.database, { [trustKey]: value(["/repos"]) });
    await vi.waitFor(() =>
      expect(editor.context.alert).toHaveBeenCalledWith(
        expect.stringContaining("Permission denied")
      )
    );
  });
});
