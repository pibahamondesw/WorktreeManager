import { SourceAnalysis } from "./semanticDiff";
import SyntaxAnalysisWorker from "./syntaxAnalysis.worker?worker";

export interface SyntaxRequest {
  id: number;
  path: string;
  text: string;
}

export interface SyntaxResponse {
  id: number;
  analysis: SourceAnalysis;
}

type Resolve = (analysis: SourceAnalysis | null) => void;

const pending = new Map<number, Resolve>();
let worker: Worker | null = null;
let nextId = 0;

function failPending() {
  for (const resolve of pending.values()) resolve(null);
  pending.clear();
  worker?.terminate();
  worker = null;
}

function syntaxWorker(): Worker {
  if (worker) return worker;
  const created = new SyntaxAnalysisWorker();
  created.onmessage = ({ data }: MessageEvent<SyntaxResponse>) => {
    pending.get(data.id)?.(data.analysis);
    pending.delete(data.id);
  };
  created.onerror = failPending;
  created.onmessageerror = failPending;
  worker = created;
  return created;
}

/** Syntax analysis for the diff viewer off the main thread; null when the worker fails. */
export function analyzeInWorker(path: string, text: string): Promise<SourceAnalysis | null> {
  return new Promise((resolve) => {
    const id = nextId++;
    try {
      const target = syntaxWorker();
      pending.set(id, resolve);
      target.postMessage({ id, path, text } satisfies SyntaxRequest);
    } catch {
      pending.delete(id);
      resolve(null);
    }
  });
}
