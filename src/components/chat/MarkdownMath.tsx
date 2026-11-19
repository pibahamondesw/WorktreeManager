import { memo } from "react";
import katex from "katex";
import "katex/dist/katex.min.css";

export default memo(function MarkdownMath({
  source,
  display,
}: {
  source: string;
  display: boolean;
}) {
  let html: string;
  try {
    html = katex.renderToString(source, { displayMode: display, trust: false, maxSize: 20 });
  } catch {
    return <span className="chat-math-source">{source}</span>;
  }
  return <span dangerouslySetInnerHTML={{ __html: html }} />;
});
