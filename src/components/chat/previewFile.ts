import { useEffect, useState } from "react";
import { chatReadFile, FileContent } from "../../services/chat";
import { SourceAnalysis } from "../../services/semanticDiff";
import { analyzeSource } from "../../services/syntaxAnalysis";

export type LoadedFile =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; path: string; content: FileContent; analysis: SourceAnalysis | null };

export function usePreviewFile(
  path: string | null,
  folders: string[],
  base: string | null,
  revision: number
): LoadedFile {
  const key = path === null ? null : `${folders.join("\n")}\n${base ?? ""}\n${path}`;
  const [loaded, setLoaded] = useState<{ key: string; state: LoadedFile }>();

  useEffect(() => {
    if (key === null) return;
    const lookup = key.split("\n");
    const filePath = lookup.pop()!;
    const baseDirectory = lookup.pop() || null;
    let stale = false;
    void chatReadFile(lookup.filter(Boolean), baseDirectory, filePath)
      .then(async (file) => ({
        kind: "ready" as const,
        ...file,
        analysis:
          file.content.kind === "text" ? await analyzeSource(file.path, file.content.text) : null,
      }))
      .catch((e) => ({ kind: "error" as const, message: String(e) }))
      .then((state) => !stale && setLoaded({ key, state }));
    return () => {
      stale = true;
    };
  }, [key, revision]);

  return loaded?.key === key ? loaded.state : { kind: "loading" };
}
