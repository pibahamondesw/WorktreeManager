// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import { MarkdownPreviewPanel } from "./MarkdownPreviewPanel";

const PATH = "/wt/a/src/main.ts";
let onDisk = "const a = 1;\n";

function renderPanel(props: { busy?: boolean } = {}) {
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
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation((command: string, args: Record<string, string>) => {
    if (command === "chat_read_file")
      return Promise.resolve({ path: PATH, content: { kind: "text", text: onDisk } });
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
});
