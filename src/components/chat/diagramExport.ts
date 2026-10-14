import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { DIAGRAM_BACKGROUND, RenderedDiagram, renderMermaidForImage } from "./mermaid";

const PIXEL_RATIO = 2;
const PNG_FILE_NAME = "mermaid-diagram.png";

function svgDataUrl({ svg, width, height }: RenderedDiagram) {
  const document = new DOMParser().parseFromString(svg, "image/svg+xml");
  const root = document.documentElement;
  root.setAttribute("width", String(width));
  root.setAttribute("height", String(height));
  root.style.maxWidth = "none";
  const markup = new XMLSerializer().serializeToString(root);
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(markup)}`;
}

function loadImage(src: string) {
  return new Promise<HTMLImageElement>((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("Could not draw the diagram"));
    image.src = src;
  });
}

async function diagramPngBase64(diagram: RenderedDiagram) {
  const image = await loadImage(svgDataUrl(diagram));
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(diagram.width * PIXEL_RATIO);
  canvas.height = Math.ceil(diagram.height * PIXEL_RATIO);
  const context = canvas.getContext("2d");
  if (!context) throw new Error("Could not draw the diagram");
  context.fillStyle = DIAGRAM_BACKGROUND;
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.drawImage(image, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL("image/png").slice("data:image/png;base64,".length);
}

/** Resolves to `false` when the user cancels the save dialog. */
export async function saveDiagramPng(source: string) {
  const diagram = await renderMermaidForImage(source);
  if (!diagram) throw new Error("Could not render the diagram");
  const base64Png = await diagramPngBase64(diagram);
  const path = await save({
    defaultPath: PNG_FILE_NAME,
    filters: [{ name: "PNG image", extensions: ["png"] }],
  });
  if (!path) return false;
  await invoke("save_png", { path, base64Png });
  return true;
}
