// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { saveDiagramPng } from "./diagramExport";
import { renderMermaidForImage } from "./mermaid";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));
vi.mock("./mermaid", () => ({
  diagramBackground: () => "#12121a",
  renderMermaidForImage: vi.fn(),
}));

const drawnImages: string[] = [];

class LoadingImage {
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  set src(value: string) {
    drawnImages.push(value);
    queueMicrotask(() => this.onload?.());
  }
}

const context = { fillStyle: "", fillRect: vi.fn(), drawImage: vi.fn() };

beforeEach(() => {
  drawnImages.length = 0;
  vi.stubGlobal("Image", LoadingImage);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
    context as unknown as CanvasRenderingContext2D
  );
  vi.spyOn(HTMLCanvasElement.prototype, "toDataURL").mockReturnValue(
    "data:image/png;base64,iVBORw0KGgo="
  );
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
  vi.mocked(save).mockReset().mockResolvedValue("/Users/me/diagram.png");
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const RENDERED = {
  svg: '<svg xmlns="http://www.w3.org/2000/svg" width="100%" style="max-width: 300px;" viewBox="0 0 300 150"></svg>',
  width: 300,
  height: 150,
};

it("saves a 2x PNG of the label-safe render to the path picked in the save dialog", async () => {
  vi.mocked(renderMermaidForImage).mockResolvedValue(RENDERED);

  expect(await saveDiagramPng("graph TD\nA-->B")).toBe(true);

  expect(renderMermaidForImage).toHaveBeenCalledWith("graph TD\nA-->B");
  const svg = decodeURIComponent(drawnImages[0].split(",")[1]);
  expect(svg).toContain('width="300"');
  expect(svg).toContain('height="150"');
  expect(context.fillStyle).toBe("#12121a");
  expect(context.drawImage).toHaveBeenCalledWith(expect.anything(), 0, 0, 600, 300);
  expect(save).toHaveBeenCalledWith(
    expect.objectContaining({ defaultPath: "mermaid-diagram.png" })
  );
  expect(invoke).toHaveBeenCalledExactlyOnceWith("save_png", {
    path: "/Users/me/diagram.png",
    base64Png: "iVBORw0KGgo=",
  });
});

it("fails without opening the dialog when the diagram cannot be rendered", async () => {
  vi.mocked(renderMermaidForImage).mockResolvedValue(null);

  await expect(saveDiagramPng("graph TD\nA-->")).rejects.toThrow("Could not render the diagram");
  expect(save).not.toHaveBeenCalled();
  expect(invoke).not.toHaveBeenCalled();
});

it("writes nothing when the save dialog is cancelled", async () => {
  vi.mocked(renderMermaidForImage).mockResolvedValue(RENDERED);
  vi.mocked(save).mockResolvedValue(null);

  expect(await saveDiagramPng("graph TD\nA-->B")).toBe(false);
  expect(invoke).not.toHaveBeenCalled();
});
