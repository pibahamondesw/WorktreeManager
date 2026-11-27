import { createContext, useContext, useEffect, useState } from "react";
import { chatReadMarkdown, MarkdownDocument } from "../../services/chat";
import { MarkdownSource } from "./markdownDocument";
import { LruCache } from "./lruCache";
import { EditorApp } from "../../types";

export interface MarkdownPreviewRequest {
  source: MarkdownSource;
  requestId?: string;
}

interface MarkdownPreviewContextValue {
  folders: string[];
  base?: string;
  open: (request: MarkdownPreviewRequest) => void;
}

export const TaskEditorContext = createContext<EditorApp | null>(null);

export const MarkdownPreviewContext = createContext<MarkdownPreviewContextValue | null>(null);

export const useMarkdownPreview = () => useContext(MarkdownPreviewContext);

type FileDocumentState =
  | { kind: "error"; message: string }
  | { kind: "ready"; document: MarkdownDocument; text: string };

interface MarkdownRefresh {
  order: number;
  state: FileDocumentState;
}

interface CachedDocument {
  order: number;
  document: MarkdownDocument;
}

export type LoadedDocument =
  | FileDocumentState
  | { kind: "loading" }
  | { kind: "ready"; document: null; text: string };

const MAX_CACHED_DOCUMENTS = 200;
const MAX_CACHED_CHARACTERS = 2_000_000;

/** The documents read last, so a card that mounts again shows its content at once instead of growing when the read returns. */
const readDocuments = new LruCache<CachedDocument>(MAX_CACHED_DOCUMENTS, MAX_CACHED_CHARACTERS);
const refreshListeners = new Set<(path: string, update: MarkdownRefresh) => void>();
const pendingRefreshes = new Map<string, number>();
let readOrder = 0;

const ready = (document: MarkdownDocument): FileDocumentState => ({
  kind: "ready",
  document,
  text: document.text,
});

function readMarkdown(folders: string[], base: string | null, path: string) {
  return chatReadMarkdown(folders, base, path).then(
    ready,
    (error): FileDocumentState => ({ kind: "error", message: String(error) })
  );
}

export async function refreshSavedMarkdown(path: string) {
  const order = ++readOrder;
  pendingRefreshes.set(path, order);
  const loaded = await readMarkdown([], null, path);
  if (pendingRefreshes.get(path) !== order) return;
  pendingRefreshes.delete(path);
  readDocuments.deleteWhere((cached) => cached.document.path === path && cached.order < order);
  const state: FileDocumentState =
    loaded.kind === "ready" && loaded.document.path !== path
      ? { kind: "error", message: "The file resolved to a different path. Reload the document." }
      : loaded;
  const update = { order, state };
  for (const listener of refreshListeners) listener(path, update);
}

const sourceKey = (source: MarkdownSource, folders: string[]) =>
  source.kind === "file" ? `${folders.join("\n")}\n${source.base ?? ""}\n${source.path}` : null;

export function useMarkdownDocument(
  source: MarkdownSource,
  folders: string[],
  revision = 0
): LoadedDocument {
  const key = sourceKey(source, folders);
  const [loaded, setLoaded] = useState<{ key: string; state: FileDocumentState }>();

  useEffect(() => {
    if (key === null) return;
    const lookup = key.split("\n");
    const path = lookup.pop()!;
    const base = lookup.pop() || null;
    let stale = false;
    const cached = readDocuments.get(key);
    let document = cached?.document;
    let appliedOrder = cached?.order ?? 0;
    const order = ++readOrder;
    let updatesDuringRead: Map<string, MarkdownRefresh> | null = new Map();
    const apply = (state: FileDocumentState, resultOrder: number) => {
      appliedOrder = resultOrder;
      if (state.kind === "ready") document = state.document;
      if ((readDocuments.get(key)?.order ?? 0) <= resultOrder) {
        if (state.kind === "ready") {
          readDocuments.set(
            key,
            { order: resultOrder, document: state.document },
            state.text.length
          );
        } else {
          readDocuments.delete(key);
        }
      }
      setLoaded({ key, state });
    };
    const applyRefresh = (current: MarkdownDocument, { order, state }: MarkdownRefresh) => {
      apply(state.kind === "ready" ? ready({ ...current, text: state.text }) : state, order);
    };
    const refresh = (savedPath: string, update: MarkdownRefresh) => {
      if (update.order > order) updatesDuringRead?.set(savedPath, update);
      if (document?.path !== savedPath || update.order < appliedOrder) return;
      applyRefresh(document, update);
    };
    refreshListeners.add(refresh);
    void readMarkdown(lookup.filter(Boolean), base, path).then((state) => {
      if (stale) return;
      const resolved = state.kind === "ready" ? state.document : document;
      const update = resolved && updatesDuringRead?.get(resolved.path);
      updatesDuringRead = null;
      if (update && resolved) applyRefresh(resolved, update);
      else apply(state, order);
    });
    return () => {
      stale = true;
      refreshListeners.delete(refresh);
    };
  }, [key, revision]);

  if (key === null) {
    return { kind: "ready", document: null, text: source.kind === "inline" ? source.text : "" };
  }
  if (loaded?.key === key) return loaded.state;
  const cached = readDocuments.get(key);
  return cached ? ready(cached.document) : { kind: "loading" };
}
