type Mermaid = typeof import("mermaid").default;

let mermaidLoader: Promise<Mermaid> | null = null;
let diagramCount = 0;

const loadMermaid = () =>
  (mermaidLoader ??= import("mermaid").then(({ default: mermaid }) => {
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      theme: "base",
      fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", "Helvetica Neue", sans-serif',
      themeVariables: {
        darkMode: true,
        background: "#12121a",
        primaryColor: "#1a1a25",
        primaryTextColor: "#e8e8ed",
        primaryBorderColor: "#3a3a48",
        secondaryColor: "#22222f",
        tertiaryColor: "#12121a",
        lineColor: "#8888a0",
        textColor: "#e8e8ed",
        fontSize: "14px",
      },
    });
    return mermaid;
  }));

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

export async function renderMermaid(source: string): Promise<RenderedDiagram | null> {
  const mermaid = await loadMermaid();
  if (!(await mermaid.parse(source, { suppressErrors: true }))) return null;
  const { svg } = await mermaid.render(`chat-mermaid-${++diagramCount}`, source);
  const size = viewBoxSize(svg);
  return size && { svg, ...size };
}
