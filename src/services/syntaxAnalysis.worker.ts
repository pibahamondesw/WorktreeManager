import { analyzeDiffSource } from "./syntaxAnalysis";
import type { SyntaxRequest, SyntaxResponse } from "./syntaxWorker";

self.onmessage = async ({ data }: MessageEvent<SyntaxRequest>) => {
  const response: SyntaxResponse = {
    id: data.id,
    analysis: await analyzeDiffSource(data.path, data.text),
  };
  self.postMessage(response);
};
