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

export type LoadedDocument =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; document: MarkdownDocument | null; text: string };

const MAX_CACHED_DOCUMENTS = 200;
const MAX_CACHED_CHARACTERS = 2_000_000;

/** The documents read last, so a card that mounts again shows its content at once instead of growing when the read returns. */
const readDocuments = new LruCache<LoadedDocument>(MAX_CACHED_DOCUMENTS, MAX_CACHED_CHARACTERS);

const sourceKey = (source: MarkdownSource, folders: string[]) =>
  source.kind === "file" ? `${(source.folders ?? folders).join("\n")}\n${source.path}` : null;

export function useMarkdownDocument(
  source: MarkdownSource,
  folders: string[],
  revision = 0
): LoadedDocument {
  const key = sourceKey(source, folders);
  const [loaded, setLoaded] = useState<{ key: string; state: LoadedDocument }>();

  useEffect(() => {
    if (key === null) return;
    const lookup = key.split("\n");
    const path = lookup.pop()!;
    let stale = false;
    void chatReadMarkdown(lookup.filter(Boolean), path)
      .then((document) => {
        const state = { kind: "ready" as const, document, text: document.text };
        readDocuments.set(key, state, state.text.length);
        return state;
      })
      .catch((e) => {
        readDocuments.delete(key);
        return { kind: "error" as const, message: String(e) };
      })
      .then((state) => !stale && setLoaded({ key, state }));
    return () => {
      stale = true;
    };
  }, [key, revision]);

  if (key === null) {
    return { kind: "ready", document: null, text: source.kind === "inline" ? source.text : "" };
  }
  if (loaded?.key !== key) return readDocuments.get(key) ?? { kind: "loading" };
  return loaded.state;
}
