import { ComponentProps, ReactNode, useEffect, useState } from "react";
import { ExtraProps } from "react-markdown";
import { ExpandIcon } from "../ui/Icons";
import { DiagramViewer } from "./DiagramViewer";
import { RenderedDiagram, renderMermaid } from "./mermaid";

type HastElement = NonNullable<ExtraProps["node"]>;
type HastChild = HastElement["children"][number];

const MERMAID_SETTLE_MS = 150;

const textContent = (node: HastChild): string =>
  node.type === "text"
    ? node.value
    : node.type === "element"
      ? node.children.map(textContent).join("")
      : "";

const isCode = (node: HastChild): node is HastElement =>
  node.type === "element" && node.tagName === "code";

function codeLanguage(code: HastElement) {
  const classes = code.properties.className;
  const language = Array.isArray(classes)
    ? classes.map(String).find((name) => name.startsWith("language-"))
    : undefined;
  return language?.slice("language-".length) ?? null;
}

export function MarkdownCodeBlock({ node, children }: ComponentProps<"pre"> & ExtraProps) {
  const code = node?.children.find(isCode);
  const language = code ? codeLanguage(code) : null;
  const block = (
    <figure className="chat-code-block">
      {language && <figcaption className="chat-code-language">{language}</figcaption>}
      <pre>{children}</pre>
    </figure>
  );
  if (code && language === "mermaid")
    return <MermaidDiagram source={textContent(code)} fallback={block} />;
  return block;
}

function MermaidDiagram({ source, fallback }: { source: string; fallback: ReactNode }) {
  const [rendered, setRendered] = useState<{ source: string; diagram: RenderedDiagram } | null>(
    null
  );
  const [previewOpen, setPreviewOpen] = useState(false);

  useEffect(() => {
    let stale = false;
    const timer = setTimeout(() => {
      renderMermaid(source)
        .then((diagram) => !stale && setRendered(diagram ? { source, diagram } : null))
        .catch(() => !stale && setRendered(null));
    }, MERMAID_SETTLE_MS);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [source]);

  if (rendered?.source !== source) return fallback;
  const { svg, width, height } = rendered.diagram;
  const openPreview = () => setPreviewOpen(true);
  return (
    <figure className="chat-mermaid">
      <div
        role="img"
        aria-label="Mermaid diagram"
        className="chat-mermaid-diagram"
        style={{
          width: `min(${width}px, 90%, calc(var(--chat-diagram-max-height) * ${width / height}))`,
          aspectRatio: `${width} / ${height}`,
        }}
        onClick={openPreview}
        dangerouslySetInnerHTML={{ __html: svg }}
      />
      <button
        type="button"
        className="chat-mermaid-expand"
        onClick={openPreview}
        aria-label="Open diagram preview"
        title="Open preview"
      >
        <ExpandIcon />
      </button>
      {previewOpen && (
        <DiagramViewer
          svg={svg}
          size={{ width, height }}
          label="Mermaid diagram preview"
          onClose={() => setPreviewOpen(false)}
        />
      )}
    </figure>
  );
}
