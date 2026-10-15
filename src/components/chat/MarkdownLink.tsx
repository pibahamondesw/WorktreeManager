import { ComponentProps, MouseEvent, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { markdownPath } from "./markdownDocument";
import { useMarkdownPreview } from "./markdownPreviewContext";

export function MarkdownLink({ href, children, title }: ComponentProps<"a">) {
  const [linkError, setLinkError] = useState<string | null>(null);
  const preview = useMarkdownPreview();
  const documentPath = preview ? markdownPath(href) : null;
  const openLink = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    if (documentPath) {
      preview?.open({ source: { kind: "file", path: documentPath } });
      return;
    }
    if (!href) return;
    setLinkError(null);
    openUrl(href).catch(() => setLinkError("Could not open the link"));
  };

  return (
    <>
      <a
        href={href}
        title={title}
        onClick={openLink}
        onAuxClick={(event) => {
          if (event.button === 1) openLink(event);
        }}
      >
        {children}
      </a>
      {linkError && (
        <span role="alert" className="text-danger">
          {" "}
          {linkError}
        </span>
      )}
    </>
  );
}
