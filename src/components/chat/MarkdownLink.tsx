import { ComponentProps, MouseEvent, useContext, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { openFileInEditor } from "../../services/openEditor";
import { fileReference, isWindowsPathLink, markdownPath } from "./markdownDocument";
import { MarkdownFileLink } from "./MarkdownFileLink";
import { TaskEditorContext, useMarkdownPreview } from "./markdownPreviewContext";

const FILE_EDITOR_LABELS = {
  cursor: "Open in Cursor",
  "vscode-web": "Open in VS Code Desktop",
  zed: "Open in Zed",
} as const;

export function MarkdownLink({ href, children, title }: ComponentProps<"a">) {
  const [linkError, setLinkError] = useState<string | null>(null);
  const preview = useMarkdownPreview();
  const editor = useContext(TaskEditorContext);
  const file = preview ? fileReference(href) : null;
  if (preview && file) {
    const documentPath = markdownPath(href);
    const editorLabel =
      editor === "cursor" || editor === "vscode-web" || editor === "zed"
        ? FILE_EDITOR_LABELS[editor]
        : null;
    return (
      <MarkdownFileLink
        href={href}
        reference={file}
        folders={preview.folders}
        editorAction={
          editor && editorLabel
            ? {
                label: editorLabel,
                run: () =>
                  openFileInEditor(editor, preview.folders, file.path, file.line, preview.base),
              }
            : undefined
        }
        onOpen={() =>
          preview.open({
            source: documentPath ? { kind: "file", path: documentPath } : { kind: "code", ...file },
          })
        }
      >
        {children}
      </MarkdownFileLink>
    );
  }

  if (href && isWindowsPathLink(href)) return <>{children}</>;

  const openLink = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
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
