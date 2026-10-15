import { createContext, useContext, useEffect, useState } from "react";
import { chatReadMarkdown, MarkdownDocument } from "../../services/chat";
import { MarkdownSource } from "./markdownDocument";

export interface MarkdownPreviewRequest {
  source: MarkdownSource;
  requestId?: string;
}

interface MarkdownPreviewContextValue {
  folders: string[];
  open: (request: MarkdownPreviewRequest) => void;
}

export const MarkdownPreviewContext = createContext<MarkdownPreviewContextValue | null>(null);

export const useMarkdownPreview = () => useContext(MarkdownPreviewContext);

export type LoadedDocument =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; document: MarkdownDocument | null; text: string };

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
      .then((document) => ({ kind: "ready" as const, document, text: document.text }))
      .catch((e) => ({ kind: "error" as const, message: String(e) }))
      .then((state) => !stale && setLoaded({ key, state }));
    return () => {
      stale = true;
    };
  }, [key, revision]);

  if (source.kind === "inline") return { kind: "ready", document: null, text: source.text };
  if (loaded?.key !== key) return { kind: "loading" };
  return loaded.state;
}
