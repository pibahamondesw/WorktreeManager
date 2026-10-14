type Mermaid = typeof import("mermaid").default;
type MermaidConfig = Parameters<Mermaid["initialize"]>[0];

export const DIAGRAM_BACKGROUND = "#12121a";

const BASE_CONFIG: MermaidConfig = {
  startOnLoad: false,
  securityLevel: "strict",
  theme: "base",
  fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", "Helvetica Neue", sans-serif',
  themeVariables: {
    darkMode: true,
    background: DIAGRAM_BACKGROUND,
    primaryColor: "#1a1a25",
    primaryTextColor: "#e8e8ed",
    primaryBorderColor: "#3a3a48",
    secondaryColor: "#22222f",
    tertiaryColor: DIAGRAM_BACKGROUND,
    lineColor: "#8888a0",
    textColor: "#e8e8ed",
    fontSize: "14px",
  },
};

// WebKit taints a canvas that draws an SVG containing <foreignObject>, so images
// are rendered with plain SVG text labels.
const IMAGE_EXPORT_CONFIG: MermaidConfig = { ...BASE_CONFIG, htmlLabels: false };

let mermaidLoader: Promise<Mermaid> | null = null;
let appliedConfig: MermaidConfig | null = null;
let renderQueue: Promise<unknown> = Promise.resolve();
let diagramCount = 0;

const loadMermaid = () =>
  (mermaidLoader ??= import("mermaid").then(({ default: mermaid }) => mermaid));

export interface RenderedDiagram {
  svg: string;
  width: number;
  height: number;
}

function viewBoxSize(svg: string) {
  const viewBox = /viewBox="([^"]+)"/.exec(svg)?.[1];
  const [, , width, height] =
    viewBox
      ?.trim()
      .split(/[\s,]+/)
      .map(Number) ?? [];
  return width > 0 && height > 0 ? { width, height } : null;
}

async function renderWith(config: MermaidConfig, source: string) {
  const mermaid = await loadMermaid();
  if (appliedConfig !== config) {
    mermaid.initialize(config);
    appliedConfig = config;
  }
  if (!(await mermaid.parse(source, { suppressErrors: true }))) return null;
  const { svg } = await mermaid.render(`chat-mermaid-${++diagramCount}`, source);
  const size = viewBoxSize(svg);
  return size && { svg, ...size };
}

function enqueueRender(config: MermaidConfig, source: string): Promise<RenderedDiagram | null> {
  const rendered = renderQueue.then(() => renderWith(config, source));
  renderQueue = rendered.catch(() => undefined);
  return rendered;
}

export const renderMermaid = (source: string) => enqueueRender(BASE_CONFIG, source);

export const renderMermaidForImage = (source: string) => enqueueRender(IMAGE_EXPORT_CONFIG, source);
