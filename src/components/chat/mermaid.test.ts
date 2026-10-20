// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from "vitest";

const mermaid = vi.hoisted(() => ({
  initialize: vi.fn<(config: { htmlLabels?: boolean; themeVariables?: object }) => void>(),
  parse: vi.fn(),
  render: vi.fn<(id: string, source: string) => Promise<{ svg: string }>>(),
}));
vi.mock("mermaid", () => ({ default: mermaid }));

import { renderMermaid, renderMermaidForImage } from "./mermaid";

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

it("renders image exports without HTML labels, one configuration at a time", async () => {
  mermaid.initialize.mockClear();
  mermaid.parse.mockResolvedValue({ diagramType: "flowchart" });
  mermaid.render.mockResolvedValue({ svg: SVG });

  await Promise.all([renderMermaidForImage("graph TD\nA-->B"), renderMermaid("graph TD\nA-->B")]);

  expect(mermaid.initialize.mock.calls.map(([config]) => config.htmlLabels)).toEqual([
    false,
    undefined,
  ]);
  expect(mermaid.initialize.mock.invocationCallOrder[0]).toBeLessThan(
    mermaid.render.mock.invocationCallOrder[0]
  );
  expect(mermaid.initialize.mock.invocationCallOrder[1]).toBeGreaterThan(
    mermaid.render.mock.invocationCallOrder[0]
  );
});

it("re-initializes with the active theme colors when the theme changes", async () => {
  mermaid.initialize.mockClear();
  mermaid.parse.mockResolvedValue({ diagramType: "flowchart" });
  mermaid.render.mockResolvedValue({ svg: SVG });
  const root = document.documentElement;
  root.style.setProperty("--color-bg-secondary", "#f2e5bc");
  root.dataset.appearance = "light";

  await renderMermaid("graph TD\nA-->B");
  await renderMermaid("graph TD\nA-->B");

  expect(mermaid.initialize).toHaveBeenCalledOnce();
  expect(mermaid.initialize.mock.calls[0][0].themeVariables).toMatchObject({
    background: "#f2e5bc",
    darkMode: false,
  });
  root.style.removeProperty("--color-bg-secondary");
  delete root.dataset.appearance;
});
