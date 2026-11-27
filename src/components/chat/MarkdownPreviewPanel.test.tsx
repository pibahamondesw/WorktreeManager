// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import { FileEdit, MarkdownPreviewPanel } from "./MarkdownPreviewPanel";

const PATH = "/wt/a/src/main.ts";
let documentCount = 0;
let documentRequest = "";
const documentPath = () => `/wt/a/${documentRequest}`;
let onDisk = "const a = 1;\n";
let documentOnDisk = "# Notes\n\nFirst draft.\n";
let documentEditable = true;

function renderPanel(
  props: {
    busy?: boolean;
    initialFileEdit?: FileEdit;
    onUnsavedFileEdit?: (edit: unknown) => void;
  } = {}
) {
  const onClose = vi.fn();
  render(
    <MarkdownPreviewPanel
      request={{ source: { kind: "code", path: "src/main.ts", line: 1 } }}
      folders={["/wt/a"]}
      onClose={onClose}
      {...props}
    />
  );
  return { panel: screen.getByRole("region", { name: "Preview of main.ts" }), onClose };
}

const contents = () => screen.getByRole("textbox", { name: "File contents" });

async function startEditing(panel: HTMLElement) {
  fireEvent.click(await within(panel).findByRole("button", { name: "Edit file" }));
  fireEvent.change(contents(), { target: { value: "const a = 2;\n" } });
}

beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn();
  onDisk = "const a = 1;\n";
  documentOnDisk = "# Notes\n\nFirst draft.\n";
  documentEditable = true;
  documentRequest = `docs/notes-${++documentCount}.md`;
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation((command: string, args: Record<string, string>) => {
    if (command === "chat_read_file")
      return Promise.resolve({ path: PATH, content: { kind: "text", text: onDisk } });
    if (command === "chat_read_markdown")
      return Promise.resolve({
        path: `${args.base ?? "/wt/a"}/${args.path}`,
        text: documentOnDisk,
        editable: documentEditable,
      });
    if (command === "chat_write_file" && args.path === documentPath()) {
      if (args.original !== documentOnDisk) return Promise.reject(`${args.path} changed on disk`);
      documentOnDisk = args.text;
      return Promise.resolve({ path: args.path, content: { kind: "text", text: args.text } });
    }
    if (command === "chat_write_file") {
      if (args.original !== onDisk) return Promise.reject("src/main.ts changed on disk");
      onDisk = args.text;
      return Promise.resolve({ path: PATH, content: { kind: "text", text: onDisk } });
    }
    return Promise.resolve(undefined);
  });
});
afterEach(cleanup);

describe("MarkdownPreviewPanel file editing", () => {
  it("saves the edited file and shows the saved contents", async () => {
    const { panel } = renderPanel();
    await startEditing(panel);

    fireEvent.click(within(panel).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(panel).toHaveTextContent("1const a = 2;"));
    expect(mocks.invoke).toHaveBeenCalledWith("chat_write_file", {
      folders: ["/wt/a"],
      path: PATH,
      original: "const a = 1;\n",
      text: "const a = 2;\n",
    });
    expect(within(panel).queryByRole("textbox", { name: "File contents" })).toBeNull();
  });

  it("keeps the file's CRLF line endings when saving", async () => {
    onDisk = "const a = 1;\r\nconst b = 1;\r\n";
    const { panel } = renderPanel();
    fireEvent.click(await within(panel).findByRole("button", { name: "Edit file" }));
    fireEvent.change(contents(), { target: { value: "const a = 1;\nconst b = 1;\n" } });
    expect(within(panel).getByRole("button", { name: "Save" })).toBeDisabled();

    fireEvent.change(contents(), { target: { value: "const a = 1;\nconst b = 2;\n" } });
    fireEvent.click(within(panel).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(onDisk).toBe("const a = 1;\r\nconst b = 2;\r\n"));
  });

  it("keeps the edit when the file changed on disk meanwhile", async () => {
    const { panel } = renderPanel();
    await startEditing(panel);
    onDisk = "const a = 3;\n";

    fireEvent.click(within(panel).getByRole("button", { name: "Save" }));

    expect(await within(panel).findByRole("alert")).toHaveTextContent("changed on disk");
    expect(contents()).toHaveValue("const a = 2;\n");
  });

  it("does not save while the agent is working", async () => {
    const { panel } = renderPanel({ busy: true });
    await startEditing(panel);

    expect(within(panel).getByRole("button", { name: "Save" })).toBeDisabled();
    expect(panel).toHaveTextContent("The agent is working");
  });

  it("asks before discarding unsaved changes and discards a cancelled edit", async () => {
    const { panel, onClose } = renderPanel();
    await startEditing(panel);

    fireEvent.keyDown(document, { key: "Escape" });
    const confirm = within(panel).getByRole("alertdialog", { name: "Unsaved changes" });
    fireEvent.click(within(confirm).getByRole("button", { name: "Keep editing" }));
    expect(onClose).not.toHaveBeenCalled();
    expect(contents()).toHaveValue("const a = 2;\n");

    fireEvent.click(within(panel).getByRole("button", { name: "Close preview" }));
    fireEvent.click(within(panel).getByRole("button", { name: "Discard" }));
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.click(within(panel).getByRole("button", { name: "Cancel" }));
    expect(within(panel).queryByRole("textbox", { name: "File contents" })).toBeNull();
    expect(panel).toHaveTextContent("1const a = 1;");
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(2);
    expect(onDisk).toBe("const a = 1;\n");
  });

  it("reports an unsaved edit until it is saved or cancelled", async () => {
    const onUnsavedFileEdit = vi.fn();
    const { panel } = renderPanel({ onUnsavedFileEdit });
    fireEvent.click(await within(panel).findByRole("button", { name: "Edit file" }));
    expect(onUnsavedFileEdit).toHaveBeenLastCalledWith(null);

    fireEvent.change(contents(), { target: { value: "const a = 2;\n" } });
    expect(onUnsavedFileEdit).toHaveBeenLastCalledWith({
      base: "const a = 1;\n",
      text: "const a = 2;\n",
      source: { kind: "code", path: "src/main.ts", line: 1 },
    });

    fireEvent.click(within(panel).getByRole("button", { name: "Cancel" }));
    expect(onUnsavedFileEdit).toHaveBeenLastCalledWith(null);

    await startEditing(panel);
    fireEvent.click(within(panel).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onDisk).toBe("const a = 2;\n"));
    expect(onUnsavedFileEdit).toHaveBeenLastCalledWith(null);
  });

  it("resumes a kept edit and saves it against the text it started from", async () => {
    const { panel } = renderPanel({
      initialFileEdit: { base: "const a = 1;\n", text: "const a = 3;\n" },
    });
    expect(contents()).toHaveValue("const a = 3;\n");

    fireEvent.click(await within(panel).findByRole("button", { name: "Save" }));

    await waitFor(() => expect(onDisk).toBe("const a = 3;\n"));
    expect(mocks.invoke).toHaveBeenCalledWith("chat_write_file", {
      folders: ["/wt/a"],
      path: PATH,
      original: "const a = 1;\n",
      text: "const a = 3;\n",
    });
  });

  it("lets a resumed edit be cancelled when the file can no longer be read", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      command === "chat_read_file"
        ? Promise.reject("File not found in the task repositories: src/main.ts")
        : Promise.resolve(undefined)
    );
    const { panel } = renderPanel({
      initialFileEdit: { base: "const a = 1;\n", text: "const a = 3;\n" },
    });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_read_file", expect.anything())
    );
    expect(within(panel).queryByRole("button", { name: "Save" })).toBeNull();

    fireEvent.click(within(panel).getByRole("button", { name: "Cancel" }));

    expect(within(panel).queryByRole("textbox", { name: "File contents" })).toBeNull();
    expect(await within(panel).findByRole("alert")).toHaveTextContent("File not found");
  });

  it("keeps a resumed edit when the file changed on disk since it started", async () => {
    onDisk = "const a = 9;\n";
    const { panel } = renderPanel({
      initialFileEdit: { base: "const a = 1;\n", text: "const a = 3;\n" },
    });

    fireEvent.click(await within(panel).findByRole("button", { name: "Save" }));

    expect(await within(panel).findByRole("alert")).toHaveTextContent("changed on disk");
    expect(contents()).toHaveValue("const a = 3;\n");
    expect(onDisk).toBe("const a = 9;\n");
  });
});

function renderDocument(props: { busy?: boolean } = {}) {
  const onClose = vi.fn();
  render(
    <MarkdownPreviewPanel
      request={{ source: { kind: "file", path: documentRequest } }}
      folders={["/wt/a"]}
      onClose={onClose}
      {...props}
    />
  );
  return { panel: screen.getByRole("region", { name: /^Preview of / }), onClose };
}

async function startEditingDocument(panel: HTMLElement) {
  fireEvent.click(await within(panel).findByRole("button", { name: "Edit file" }));
  fireEvent.change(contents(), { target: { value: "# Notes\n\nSecond draft.\n" } });
}

describe("MarkdownPreviewPanel Markdown editing", () => {
  it("edits the document as text and renders it again once saved", async () => {
    const { panel } = renderDocument();
    await startEditingDocument(panel);
    expect(within(panel).queryByRole("heading", { level: 1, name: "Notes" })).toBeNull();

    fireEvent.click(within(panel).getByRole("button", { name: "Save" }));

    await waitFor(() => expect(panel).toHaveTextContent("Second draft."));
    expect(within(panel).getByRole("heading", { level: 1, name: "Notes" })).toBeInTheDocument();
    expect(within(panel).queryByRole("textbox", { name: "File contents" })).toBeNull();
    expect(mocks.invoke).toHaveBeenCalledWith("chat_write_file", {
      folders: ["/wt/a"],
      path: documentPath(),
      original: "# Notes\n\nFirst draft.\n",
      text: "# Notes\n\nSecond draft.\n",
    });
  });

  it("does not offer editing a document outside the task repositories", async () => {
    documentEditable = false;
    const { panel } = renderDocument();

    await within(panel).findByText("First draft.");

    expect(within(panel).queryByRole("button", { name: "Edit file" })).toBeNull();
  });

  it("keeps the edit when the document changed on disk meanwhile", async () => {
    const { panel } = renderDocument();
    await startEditingDocument(panel);
    documentOnDisk = "# Notes\n\nAgent draft.\n";

    fireEvent.click(within(panel).getByRole("button", { name: "Save" }));

    expect(await within(panel).findByRole("alert")).toHaveTextContent("changed on disk");
    expect(contents()).toHaveValue("# Notes\n\nSecond draft.\n");
    expect(documentOnDisk).toBe("# Notes\n\nAgent draft.\n");
  });

  it("does not save the document while the agent is working", async () => {
    const { panel } = renderDocument({ busy: true });
    await startEditingDocument(panel);

    expect(within(panel).getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("asks before discarding unsaved changes to the document", async () => {
    const { panel, onClose } = renderDocument();
    await startEditingDocument(panel);

    fireEvent.keyDown(document, { key: "Escape" });
    expect(within(panel).getByRole("alertdialog", { name: "Unsaved changes" })).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.click(within(panel).getByRole("button", { name: "Discard" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(documentOnDisk).toBe("# Notes\n\nFirst draft.\n");
  });

  it("keeps the outline pointing at the document after an edit is cancelled", async () => {
    documentOnDisk = "# Notes\n\n## Steps\n\n## Risks\n";
    const { panel } = renderDocument();
    fireEvent.click(await within(panel).findByRole("button", { name: "Edit file" }));
    fireEvent.click(within(panel).getByRole("button", { name: "Cancel" }));

    const outline = within(panel).getByRole("navigation", { name: "Outline" });
    fireEvent.click(within(outline).getByRole("button", { name: "Risks" }));

    const scrolled = vi.mocked(Element.prototype.scrollIntoView).mock.contexts[0] as Element;
    expect(scrolled.isConnected).toBe(true);
    expect(scrolled).toHaveTextContent("Risks");
  });

  it("resolves a linked document against the directory of the one linking it", async () => {
    documentOnDisk = "# Notes\n\nSee [the next notes](next.md).\n";
    const { panel } = renderDocument();

    fireEvent.click(await within(panel).findByRole("link", { name: "the next notes" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("chat_read_markdown", {
        folders: ["/wt/a"],
        base: "/wt/a/docs",
        path: "next.md",
      })
    );
  });
});
