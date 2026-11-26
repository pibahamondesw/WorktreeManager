// @vitest-environment jsdom
import { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { openUrl } from "@tauri-apps/plugin-opener";
import { invoke } from "@tauri-apps/api/core";
import { EditorApp } from "../../types";
import { MarkdownLink } from "./MarkdownLink";
import {
  MarkdownPreviewContext,
  MarkdownPreviewRequest,
  TaskEditorContext,
} from "./markdownPreviewContext";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn(() => Promise.resolve()) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));

const open = vi.fn<(request: MarkdownPreviewRequest) => void>();
const writeText = vi.fn<(text: string) => Promise<void>>();

beforeEach(() => {
  writeText.mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function renderLink(
  href: string,
  text?: ReactNode,
  editor: EditorApp | null = null,
  base?: string
) {
  return render(
    <TaskEditorContext.Provider value={editor}>
      <MarkdownPreviewContext.Provider value={{ folders: ["/wt/api", "/wt/web"], base, open }}>
        <MarkdownLink href={href}>{text}</MarkdownLink>
      </MarkdownPreviewContext.Provider>
    </TaskEditorContext.Provider>
  );
}

const openMenu = (href = "/wt/web/src/app.ts#L12") => {
  renderLink(href);
  fireEvent.contextMenu(screen.getByRole("link"), { clientX: 40, clientY: 60 });
  return screen.getByRole("menu", { name: "File path" });
};

it("shows a file path as a chip with its type icon, repository-relative path and line", () => {
  renderLink("/wt/web/src/app.ts#L12", <code>app.ts:12</code>);

  const chip = screen.getByRole("link", { name: "src/app.ts:12" });
  expect(chip).toHaveClass("chat-file-link");
  expect(chip).toHaveAttribute("title", "/wt/web/src/app.ts");
  expect(chip).toHaveAttribute("aria-expanded", "false");
  expect(chip.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
});

it("keeps the words the agent linked when they are not the path", () => {
  renderLink("src/retry.ts#L40", ["the ", <em key="em">retry</em>, " guard"]);

  expect(screen.getByRole("link", { name: "the retry guard" })).toHaveAttribute(
    "title",
    "src/retry.ts"
  );
});

it("opens code files in the code preview at the cited line", () => {
  renderLink("src/app.ts:12");

  fireEvent.click(screen.getByRole("link", { name: "src/app.ts:12" }));

  expect(open).toHaveBeenCalledWith({ source: { kind: "code", path: "src/app.ts", line: 12 } });
});

it("opens Markdown files in the document preview", () => {
  renderLink("docs/plan.md");

  fireEvent.click(screen.getByRole("link", { name: "docs/plan.md" }));

  expect(open).toHaveBeenCalledWith({ source: { kind: "file", path: "docs/plan.md" } });
});

it("opens the file on middle click and ignores other auxiliary buttons", () => {
  renderLink("src/app.ts");
  const chip = screen.getByRole("link");

  fireEvent(chip, new MouseEvent("auxclick", { bubbles: true, button: 2 }));
  expect(open).not.toHaveBeenCalled();

  fireEvent(chip, new MouseEvent("auxclick", { bubbles: true, button: 1 }));
  expect(open).toHaveBeenCalledOnce();
});

it("keeps web links as plain links", () => {
  renderLink("https://example.com/src/app.ts", "the docs");

  fireEvent.click(screen.getByRole("link", { name: "the docs" }));

  expect(openUrl).toHaveBeenCalledWith("https://example.com/src/app.ts");
  expect(open).not.toHaveBeenCalled();
});

it("copies the relative path with its line from the context menu", () => {
  const menu = openMenu();

  expect(menu).toHaveStyle({ left: "40px", top: "60px" });
  expect(screen.getByRole("link")).toHaveAttribute("aria-expanded", "true");
  expect(screen.getByRole("link")).toHaveAttribute("aria-controls", menu.id);
  expect(screen.getByRole("menuitem", { name: "Copy relative path" })).toHaveFocus();
  fireEvent.click(screen.getByRole("menuitem", { name: "Copy relative path" }));

  expect(writeText).toHaveBeenCalledWith("src/app.ts:12");
  expect(screen.queryByRole("menu")).toBeNull();
  expect(screen.getByRole("link")).toHaveFocus();
});

it("copies the path as written from the context menu", () => {
  openMenu();

  fireEvent.click(screen.getByRole("menuitem", { name: "Copy path" }));

  expect(writeText).toHaveBeenCalledWith("/wt/web/src/app.ts:12");
});

it("reports when the path cannot be copied", async () => {
  writeText.mockRejectedValue(new Error("denied"));
  openMenu();

  fireEvent.click(screen.getByRole("menuitem", { name: "Copy path" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("Could not copy the path");
});

it("opens a chip in the selected editor at its line", () => {
  renderLink("src/app.ts#L12", undefined, "cursor");
  fireEvent.contextMenu(screen.getByRole("link"));
  const action = screen.getByRole("menuitem", { name: "Open in Cursor" });
  expect(action).toHaveAttribute("title", "Open in Cursor");

  fireEvent.click(action);

  expect(invoke).toHaveBeenCalledWith("open_editor_file", {
    editor: "cursor",
    folders: ["/wt/api", "/wt/web"],
    path: "src/app.ts",
    line: 12,
    base: undefined,
  });
  expect(open).not.toHaveBeenCalled();
  expect(screen.queryByRole("menu")).toBeNull();
  expect(screen.getByRole("link")).toHaveFocus();
});

it("passes the enclosing document's directory separately from the repositories", () => {
  renderLink("details.md", undefined, "zed", "/wt/web/docs");
  fireEvent.contextMenu(screen.getByRole("link"));

  fireEvent.click(screen.getByRole("menuitem", { name: "Open in Zed" }));

  expect(invoke).toHaveBeenCalledWith("open_editor_file", {
    editor: "zed",
    folders: ["/wt/api", "/wt/web"],
    path: "details.md",
    line: undefined,
    base: "/wt/web/docs",
  });
});

it("shows the editor launch error on the chip", async () => {
  vi.mocked(invoke).mockRejectedValueOnce("File not found: missing.ts");
  renderLink("missing.ts", undefined, "vscode-web");
  fireEvent.contextMenu(screen.getByRole("link"));

  fireEvent.click(screen.getByRole("menuitem", { name: "Open in VS Code Desktop" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("File not found: missing.ts");
});

it("keeps copy actions available when the selection is an agent", () => {
  renderLink("src/app.ts", undefined, "codex");
  fireEvent.contextMenu(screen.getByRole("link"));

  expect(screen.getByRole("menuitem", { name: "Copy path" })).toBeInTheDocument();
  expect(screen.queryByRole("menuitem", { name: /Open in/ })).toBeNull();
});

it.each([{ key: "ContextMenu" }, { key: "F10", shiftKey: true }])(
  "opens the menu below the chip with %o",
  (keys) => {
    renderLink("/wt/web/src/app.ts");
    const chip = screen.getByRole("link");

    fireEvent.keyDown(chip, { key: "F10" });
    expect(screen.queryByRole("menu")).toBeNull();

    fireEvent.keyDown(chip, keys);
    expect(screen.getByRole("menu", { name: "File path" })).toBeInTheDocument();
  }
);

it("moves between items with the arrow keys", () => {
  const menu = openMenu();
  const [relative, full] = screen.getAllByRole("menuitem");

  fireEvent.keyDown(menu, { key: "ArrowDown" });
  expect(full).toHaveFocus();
  fireEvent.keyDown(menu, { key: "ArrowDown" });
  expect(relative).toHaveFocus();
  fireEvent.keyDown(menu, { key: "ArrowUp" });
  expect(full).toHaveFocus();
  fireEvent.keyDown(menu, { key: "Enter" });
  expect(menu).toBeInTheDocument();
});

it("closes on Tab and lets focus continue from the chip", () => {
  const menu = openMenu();

  fireEvent.keyDown(menu, { key: "Tab" });

  expect(screen.queryByRole("menu")).toBeNull();
  expect(screen.getByRole("link")).toHaveFocus();
});

it("closes on Escape without letting the preview close too, and returns focus to the chip", () => {
  const menu = openMenu();
  const outer = vi.fn((event: KeyboardEvent) => event.defaultPrevented);
  document.addEventListener("keydown", outer);

  fireEvent.keyDown(menu, { key: "Escape" });

  document.removeEventListener("keydown", outer);
  expect(screen.queryByRole("menu")).toBeNull();
  expect(screen.getByRole("link")).toHaveFocus();
  expect(outer).not.toHaveBeenCalled();
});

it.each([
  ["a click outside", () => fireEvent.pointerDown(document.body)],
  ["scrolling", () => fireEvent.scroll(window)],
  ["resizing", () => fireEvent(window, new Event("resize"))],
])("dismisses the menu on %s", async (_, dismiss) => {
  const menu = openMenu();

  fireEvent.pointerDown(menu);
  expect(screen.getByRole("menu")).toBeInTheDocument();
  dismiss();

  await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
});
