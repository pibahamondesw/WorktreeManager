import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SyntaxRequest } from "./syntaxWorker";

const workers = vi.hoisted(() => ({
  created: [] as FakeWorker[],
  failToCreate: false,
}));

interface FakeWorker {
  posted: SyntaxRequest[];
  terminated: boolean;
  onmessage: ((event: MessageEvent) => void) | null;
  onerror: (() => void) | null;
}

vi.mock("./syntaxAnalysis.worker?worker", () => ({
  default: class {
    posted: SyntaxRequest[] = [];
    terminated = false;
    onmessage = null;
    onerror = null;
    constructor() {
      if (workers.failToCreate) throw new ReferenceError("Worker is not defined");
      workers.created.push(this);
    }
    postMessage(request: SyntaxRequest) {
      this.posted.push(request);
    }
    terminate() {
      this.terminated = true;
    }
  },
}));

const analysis = { tokens: [{ from: 0, to: 1 }], highlights: [] };

const load = async () => {
  vi.resetModules();
  return (await import("./syntaxWorker")).analyzeInWorker;
};

beforeEach(() => {
  workers.created = [];
  workers.failToCreate = false;
});

describe("analyzeInWorker", () => {
  it("matches each answer to its request on a single shared worker", async () => {
    const analyzeInWorker = await load();

    const first = analyzeInWorker("a.ts", "a");
    const second = analyzeInWorker("b.ts", "b");
    const [worker] = workers.created;
    const [firstRequest, secondRequest] = worker.posted;
    worker.onmessage!(new MessageEvent("message", { data: { id: secondRequest.id, analysis } }));
    worker.onmessage!(
      new MessageEvent("message", { data: { id: firstRequest.id, analysis: { ...analysis } } })
    );

    expect(workers.created).toHaveLength(1);
    expect(worker.posted.map(({ path, text }) => [path, text])).toEqual([
      ["a.ts", "a"],
      ["b.ts", "b"],
    ]);
    expect(await second).toBe(analysis);
    expect(await first).toEqual(analysis);
  });

  it("resolves pending requests with null and starts a new worker after a crash", async () => {
    const analyzeInWorker = await load();

    const pending = analyzeInWorker("a.ts", "a");
    const crashed = workers.created[0];
    crashed.onerror!();

    expect(await pending).toBeNull();
    expect(crashed.terminated).toBe(true);
    void analyzeInWorker("a.ts", "a");
    expect(workers.created).toHaveLength(2);
  });

  it("resolves with null when workers are unavailable", async () => {
    workers.failToCreate = true;
    const analyzeInWorker = await load();

    expect(await analyzeInWorker("a.ts", "a")).toBeNull();
  });
});

describe("syntax analysis worker", () => {
  it("answers each request with the diff analysis of its source", async () => {
    const postMessage = vi.fn();
    vi.stubGlobal("self", { postMessage });
    vi.resetModules();
    await import("./syntaxAnalysis.worker");

    await self.onmessage!(
      new MessageEvent("message", { data: { id: 7, path: "notes.txt", text: "a b" } })
    );

    expect(postMessage).toHaveBeenCalledWith({
      id: 7,
      analysis: {
        tokens: [
          { from: 0, to: 1 },
          { from: 2, to: 3 },
        ],
        highlights: [],
      },
    });
    vi.unstubAllGlobals();
  });
});
