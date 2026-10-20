type Mermaid = typeof import("mermaid").default;
type MermaidConfig = Parameters<Mermaid["initialize"]>[0];

const cssVar = (name: string, fallback: string) =>
  getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;

export const diagramBackground = () => cssVar("--color-bg-secondary", "#12121a");

function themedConfig(): MermaidConfig {
  const background = diagramBackground();
  const text = cssVar("--color-text-primary", "#e8e8ed");
  return {
    startOnLoad: false,
    securityLevel: "strict",
    theme: "base",
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", "Helvetica Neue", sans-serif',
    themeVariables: {
      darkMode: document.documentElement.dataset.appearance !== "light",
      background,
      primaryColor: cssVar("--color-bg-tertiary", "#1a1a25"),
      primaryTextColor: text,
      primaryBorderColor: cssVar("--color-border-light", "#3a3a48"),
      secondaryColor: cssVar("--color-bg-hover", "#22222f"),
      tertiaryColor: background,
      lineColor: cssVar("--color-text-secondary", "#8888a0"),
      textColor: text,
      fontSize: "14px",
    },
  };
}

// WebKit taints a canvas that draws an SVG containing <foreignObject>, so images
// are rendered with plain SVG text labels.
const imageExportConfig = (): MermaidConfig => ({ ...themedConfig(), htmlLabels: false });

let mermaidLoader: Promise<Mermaid> | null = null;
let appliedConfig: string | null = null;
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

async function renderWith(buildConfig: () => MermaidConfig, source: string) {
  const mermaid = await loadMermaid();
  const config = buildConfig();
  const configKey = JSON.stringify(config);
  if (appliedConfig !== configKey) {
    mermaid.initialize(config);
    appliedConfig = configKey;
  }
  if (!(await mermaid.parse(source, { suppressErrors: true }))) return null;
  const { svg } = await mermaid.render(`chat-mermaid-${++diagramCount}`, source);
  const size = viewBoxSize(svg);
  return size && { svg, ...size };
}

function enqueueRender(
  buildConfig: () => MermaidConfig,
  source: string
): Promise<RenderedDiagram | null> {
  const rendered = renderQueue.then(() => renderWith(buildConfig, source));
  renderQueue = rendered.catch(() => undefined);
  return rendered;
}

export const renderMermaid = (source: string) => enqueueRender(themedConfig, source);

export const renderMermaidForImage = (source: string) => enqueueRender(imageExportConfig, source);
