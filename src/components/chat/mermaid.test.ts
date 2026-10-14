import { beforeEach, expect, it, vi } from "vitest";

const mermaid = vi.hoisted(() => ({
  initialize: vi.fn(),
  parse: vi.fn(),
  render: vi.fn<(id: string, source: string) => Promise<{ svg: string }>>(),
}));
vi.mock("mermaid", () => ({ default: mermaid }));

import { renderMermaid } from "./mermaid";

beforeEach(() => {
  mermaid.parse.mockReset();
  mermaid.render.mockReset();
});

const SVG = '<svg viewBox="-8 -8 320 640"></svg>';

it("renders valid diagrams with a strict, once-initialized mermaid", async () => {
  mermaid.parse.mockResolvedValue({ diagramType: "flowchart" });
  mermaid.render.mockResolvedValue({ svg: SVG });

  expect(await renderMermaid("graph TD\nA-->B")).toEqual({ svg: SVG, width: 320, height: 640 });
  expect(await renderMermaid("graph TD\nB-->C")).toEqual({ svg: SVG, width: 320, height: 640 });

  expect(mermaid.initialize).toHaveBeenCalledExactlyOnceWith(
    expect.objectContaining({ startOnLoad: false, securityLevel: "strict" })
  );
  const ids = mermaid.render.mock.calls.map(([id]) => id);
  expect(new Set(ids).size).toBe(2);
});

it("skips rendering diagrams that do not parse", async () => {
  mermaid.parse.mockResolvedValue(false);

  expect(await renderMermaid("graph TD\nA-->")).toBeNull();
  expect(mermaid.parse).toHaveBeenCalledWith("graph TD\nA-->", { suppressErrors: true });
  expect(mermaid.render).not.toHaveBeenCalled();
});

it("skips diagrams without a measurable viewBox", async () => {
  mermaid.parse.mockResolvedValue({ diagramType: "flowchart" });
  mermaid.render.mockResolvedValue({ svg: '<svg viewBox="0 0 0 0"></svg>' });

  expect(await renderMermaid("graph TD\nA-->B")).toBeNull();
});
