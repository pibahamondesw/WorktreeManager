function synchronizeWorkspaceTrust(config) {
  if (window !== window.top || location.origin !== config.origin) return;

  const databaseName = "vscode-web-state-db-global-shared";
  const storeName = "ItemTable";
  const trustKey = "content.trust.model.key";
  const documentId = crypto.randomUUID();
  const open = indexedDB.open.bind(indexedDB);
  const put = IDBObjectStore.prototype.put;
  const remove = IDBObjectStore.prototype.delete;
  const pending = new Map();
  const transactions = new WeakMap();
  let nextId = 0;
  let database;
  let revision = -1;
  let knownPaths = new Set();
  let latestSnapshot;
  let pendingChanges = 0;
  let queue = Promise.resolve();

  function report(error) {
    console.error("Unable to synchronize workspace trust", error);
    window.alert(`Unable to synchronize workspace trust: ${error.message}`);
  }

  function enqueue(operation) {
    queue = queue.then(operation).catch(report);
    return queue;
  }

  function request(change) {
    return new Promise((resolve, reject) => {
      const id = ++nextId;
      const timeout = setTimeout(() => {
        pending.delete(id);
        reject(new Error("WorktreeManager did not respond. Reopen the editor to retry."));
      }, 10000);
      pending.set(id, { resolve, reject, timeout });
      const target = new URL(config.endpoint, config.origin);
      target.searchParams.set("token", config.token);
      target.searchParams.set("message", JSON.stringify({ id, documentId, change }));
      location.href = target.href;
    });
  }

  window.__worktreeTrustReceive = ({ id, documentId: sourceDocument, snapshot, error }) => {
    if (id !== undefined && sourceDocument !== documentId) return;
    const response = pending.get(id);
    if (response) {
      clearTimeout(response.timeout);
      pending.delete(id);
      if (error) response.reject(new Error(error));
      else response.resolve(snapshot);
    } else if (snapshot) {
      if (!latestSnapshot || snapshot.revision > latestSnapshot.revision) {
        latestSnapshot = snapshot;
      }
      if (database && revision >= 0) enqueue(() => applySnapshot(snapshot));
    }
  };

  function entries(value) {
    const state = value === undefined ? { uriTrustInfo: [] } : JSON.parse(value);
    if (!Array.isArray(state.uriTrustInfo)) throw new Error("Invalid VS Code workspace trust data");
    return state.uriTrustInfo;
  }

  function localEntry({ uri }) {
    return (
      (uri.scheme === "file" && !uri.authority) ||
      (uri.scheme === "vscode-remote" && uri.authority === location.host)
    );
  }

  function readPaths(value) {
    return new Set(
      entries(value)
        .filter((entry) => entry.trusted === true && localEntry(entry))
        .map(({ uri }) => uri.path.replace(/\/+$/, "") || "/")
        .filter((path) => path !== config.workspace)
    );
  }

  function trustValue(paths, unrelated) {
    const trusted = new Set(paths);
    const includes = (folder) =>
      [...trusted].some(
        (path) => folder === path || folder.startsWith(path === "/" ? "/" : `${path}/`)
      );
    if (config.folders.length > 0 && config.folders.every(includes)) {
      trusted.add(config.workspace);
    }
    return JSON.stringify({
      uriTrustInfo: [
        ...unrelated,
        ...[...trusted].map((path) => ({
          uri: { scheme: "vscode-remote", authority: location.host, path },
          trusted: true,
        })),
      ],
    });
  }

  async function applySnapshot(snapshot) {
    if (!latestSnapshot || snapshot.revision > latestSnapshot.revision) latestSnapshot = snapshot;
    if (pendingChanges > 0) return;
    snapshot = latestSnapshot;
    if (snapshot.revision <= revision) return;
    let value;
    await new Promise((resolve, reject) => {
      const transaction = database.transaction(storeName, "readwrite");
      transaction.oncomplete = resolve;
      transaction.onabort = () => reject(transaction.error ?? new Error("Trust update aborted"));
      transaction.onerror = () => reject(transaction.error);
      const store = transaction.objectStore(storeName);
      const previous = store.get(trustKey);
      previous.onsuccess = () => {
        try {
          value = trustValue(
            snapshot.paths,
            entries(previous.result).filter((entry) => !localEntry(entry))
          );
          put.call(store, value, trustKey);
        } catch (error) {
          transaction.abort();
          reject(error);
        }
      };
    });
    knownPaths = new Set(snapshot.paths);
    revision = snapshot.revision;
    const channel = new BroadcastChannel(databaseName);
    channel.postMessage({ changed: new Map([[trustKey, value]]) });
    channel.close();
  }

  async function initialize(connection) {
    database = connection;
    const value = await new Promise((resolve, reject) => {
      const transaction = database.transaction(storeName, "readonly");
      const result = transaction.objectStore(storeName).get(trustKey);
      transaction.oncomplete = () => resolve(result.result);
      transaction.onabort = () => reject(transaction.error ?? new Error("Trust read aborted"));
      transaction.onerror = () => reject(transaction.error);
    });
    const snapshot = await request({ operation: "initialize", paths: [...readPaths(value)] });
    await applySnapshot(snapshot);
    if (latestSnapshot) await applySnapshot(latestSnapshot);
  }

  indexedDB.open = function (name, ...args) {
    const result = open(name, ...args);
    if (name === databaseName && !database) {
      const initializeBeforeOpen = (event) => {
        result.removeEventListener("success", initializeBeforeOpen);
        event.stopImmediatePropagation();
        enqueue(() => initialize(result.result)).finally(() => {
          result.dispatchEvent(new Event("success"));
        });
      };
      result.addEventListener("success", initializeBeforeOpen);
    }
    return result;
  };

  function trackChange(store, key, value) {
    if (
      store.name !== storeName ||
      store.transaction.db.name !== databaseName ||
      key !== trustKey
    ) {
      return;
    }
    let change = transactions.get(store.transaction);
    if (!change) {
      change = { value };
      transactions.set(store.transaction, change);
      store.transaction.addEventListener("complete", () => {
        try {
          const after = readPaths(change.value);
          const added = [...after].filter((path) => !knownPaths.has(path));
          const removed = [...knownPaths].filter((path) => !after.has(path));
          knownPaths = after;
          if (!added.length && !removed.length) return;
          pendingChanges += 1;
          enqueue(async () => {
            let snapshot;
            try {
              snapshot = await request({ operation: "update", added, removed });
            } finally {
              pendingChanges -= 1;
            }
            await applySnapshot(snapshot);
          });
        } catch (error) {
          report(error);
        }
      });
    }
    change.value = value;
  }

  IDBObjectStore.prototype.put = function (value, key) {
    const result = put.call(this, value, key);
    trackChange(this, key, value);
    return result;
  };

  IDBObjectStore.prototype.delete = function (key) {
    const result = remove.call(this, key);
    trackChange(this, key, undefined);
    return result;
  };
}
