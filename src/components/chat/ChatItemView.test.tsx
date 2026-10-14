// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ChatItemView, MarkdownText } from "./ChatItemView";
import { renderMermaid } from "./mermaid";
import { saveDiagramPng } from "./diagramExport";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("./mermaid", () => ({ renderMermaid: vi.fn() }));
vi.mock("./diagramExport", () => ({ saveDiagramPng: vi.fn() }));
vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion: vi.fn() }));

beforeEach(() => vi.mocked(openUrl).mockReset().mockResolvedValue(undefined));
afterEach(cleanup);

it.each([
  [
    "[Documentation](https://example.com/docs?tab=api#usage)",
    "Documentation",
    "https://example.com/docs?tab=api#usage",
  ],
  ["https://example.com", "https://example.com", "https://example.com"],
])("opens Markdown links externally: %s", (text, label, url) => {
  render(<MarkdownText text={text} />);

  expect(fireEvent.click(screen.getByRole("link", { name: label }))).toBe(false);
  expect(openUrl).toHaveBeenCalledExactlyOnceWith(url);
});

it.each([{ ctrlKey: true }, { metaKey: true }, { shiftKey: true }])(
  "prevents webview navigation for modified clicks: %s",
  (modifiers) => {
    render(<MarkdownText text="[Documentation](https://example.com)" />);

    expect(fireEvent.click(screen.getByRole("link"), modifiers)).toBe(false);
    expect(openUrl).toHaveBeenCalledExactlyOnceWith("https://example.com");
  }
);

it("opens middle clicks externally and ignores right clicks", () => {
  render(<MarkdownText text="[Documentation](https://example.com)" />);
  const link = screen.getByRole("link");

  fireEvent(link, new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 2 }));
  expect(openUrl).not.toHaveBeenCalled();
  expect(
    fireEvent(link, new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }))
  ).toBe(false);
  expect(openUrl).toHaveBeenCalledExactlyOnceWith("https://example.com");
});

it("keeps the conversation visible when opening fails and allows retrying", async () => {
  vi.mocked(openUrl).mockRejectedValueOnce(new Error("Browser unavailable"));
  render(<MarkdownText text="Conversation with [Documentation](https://example.com)" />);
  const link = screen.getByRole("link");

  expect(fireEvent.click(link)).toBe(false);
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not open the link");
  expect(screen.getByText(/Conversation with/)).toBeVisible();

  expect(fireEvent.click(link)).toBe(false);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(openUrl).toHaveBeenCalledTimes(2);
});

it("does not open unsafe URLs stripped by Markdown", () => {
  render(<MarkdownText text="[Unsafe](javascript:alert%281%29)" />);

  expect(fireEvent.click(screen.getByText("Unsafe"))).toBe(false);
  expect(openUrl).not.toHaveBeenCalled();
});

it("labels and highlights fenced code by language", () => {
  render(<MarkdownText text={'```ruby\ndef hola\n  "hello"\nend\n```'} />);

  expect(screen.getByRole("figure")).toHaveTextContent(/^ruby/);
  expect(screen.getByText("def")).toHaveClass("hljs-keyword");
  expect(screen.getByText('"hello"')).toHaveClass("hljs-string");
});

it("leaves unlabeled code blocks without a language header", () => {
  render(<MarkdownText text={"```\nplain output\n```"} />);

  expect(screen.getByRole("figure")).toHaveTextContent(/^plain output$/);
});

it("renders user messages as Markdown while keeping their line breaks", () => {
  const { container } = render(
    <ChatItemView
      item={{ id: "u", kind: "user", text: 'First line\nsecond line\n```json\n{"a": 1}\n```' }}
    />
  );

  expect(container.querySelector(".chat-user-message p")?.textContent).toBe(
    "First line\nsecond line"
  );
  expect(screen.getByText('"a"')).toHaveClass("hljs-attr");
});

const DIAGRAM = { svg: '<svg data-testid="diagram"></svg>', width: 300, height: 600 };

it("draws valid mermaid diagrams and keeps the source for invalid ones", async () => {
  vi.mocked(renderMermaid).mockImplementation(async (source) =>
    source.includes("A-->B") ? DIAGRAM : null
  );
  const { rerender } = render(<MarkdownText text={"```mermaid\ngraph TD\nA-->B\n```"} />);

  expect(await screen.findByRole("img", { name: "Mermaid diagram" })).toContainElement(
    screen.getByTestId("diagram")
  );

  rerender(<MarkdownText text={"```mermaid\ngraph TD\nA-->\n```"} />);
  expect(await screen.findByText(/A-->/)).toBeInTheDocument();
  await new Promise((resolve) => setTimeout(resolve, 200));
  expect(screen.queryByRole("img", { name: "Mermaid diagram" })).not.toBeInTheDocument();
  expect(renderMermaid).toHaveBeenLastCalledWith("graph TD\nA-->\n");
});

it("sizes mermaid diagrams to their aspect ratio without upscaling them", async () => {
  vi.mocked(renderMermaid).mockResolvedValue(DIAGRAM);
  render(<MarkdownText text={"```mermaid\ngraph TD\nA-->B\n```"} />);

  const diagram = await screen.findByRole("img", { name: "Mermaid diagram" });
  expect(diagram.style.aspectRatio).toBe("300 / 600");
  expect(diagram.style.width).toBe("min(300px, 90%, calc(var(--chat-diagram-max-height) * 0.5))");
});

it("opens mermaid diagrams in a zoomable preview and closes it with Escape", async () => {
  vi.mocked(renderMermaid).mockResolvedValue(DIAGRAM);
  render(<MarkdownText text={"```mermaid\ngraph TD\nA-->B\n```"} />);

  fireEvent.click(await screen.findByRole("button", { name: "Open diagram preview" }));
  const preview = screen.getByRole("dialog", { name: "Mermaid diagram preview" });
  expect(preview).toHaveFocus();
  expect(preview).toContainElement(screen.getAllByTestId("diagram")[1]);

  fireEvent.click(screen.getByRole("button", { name: "Actual size" }));
  expect(screen.getByRole("status")).toHaveTextContent("100%");
  fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
  expect(screen.getByRole("status")).toHaveTextContent("125%");
  fireEvent.keyDown(preview, { key: "-" });
  expect(screen.getByRole("status")).toHaveTextContent("100%");
  fireEvent.change(screen.getByRole("slider", { name: "Zoom level" }), { target: { value: "2" } });
  expect(screen.getByRole("status")).toHaveTextContent("400%");

  fireEvent.keyDown(preview, { key: "Escape" });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("zooms the preview with pinch gestures and limits each mouse wheel step", async () => {
  vi.mocked(renderMermaid).mockResolvedValue(DIAGRAM);
  render(<MarkdownText text={"```mermaid\ngraph TD\nA-->B\n```"} />);
  fireEvent.click(await screen.findByRole("button", { name: "Open diagram preview" }));
  const preview = screen.getByRole("dialog", { name: "Mermaid diagram preview" });
  fireEvent.click(screen.getByRole("button", { name: "Actual size" }));

  fireEvent.wheel(preview, { deltaY: -100, metaKey: true });
  expect(screen.getByRole("status")).toHaveTextContent("122%");

  fireEvent(preview, new Event("gesturestart", { cancelable: true }));
  fireEvent(preview, Object.assign(new Event("gesturechange", { cancelable: true }), { scale: 2 }));
  expect(screen.getByRole("status")).toHaveTextContent("244%");

  fireEvent.wheel(preview, { deltaY: 100 });
  expect(screen.getByRole("status")).toHaveTextContent("244%");
});

it("copies the diagram source from the chat", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  vi.mocked(renderMermaid).mockResolvedValue(DIAGRAM);
  render(<MarkdownText text={"```mermaid\ngraph TD\nA-->B\n```"} />);

  fireEvent.click(await screen.findByRole("button", { name: "Copy Mermaid source" }));

  expect(writeText).toHaveBeenCalledExactlyOnceWith("graph TD\nA-->B\n");
  expect(await screen.findByRole("button", { name: "Mermaid source copied" })).toBeInTheDocument();
  vi.unstubAllGlobals();
});

it("downloads the diagram from the preview and reports failures", async () => {
  vi.mocked(renderMermaid).mockResolvedValue(DIAGRAM);
  vi.mocked(saveDiagramPng).mockResolvedValueOnce(true).mockRejectedValueOnce(new Error("disk"));
  render(<MarkdownText text={"```mermaid\ngraph TD\nA-->B\n```"} />);
  fireEvent.click(await screen.findByRole("button", { name: "Open diagram preview" }));
  const preview = within(screen.getByRole("dialog"));

  fireEvent.click(preview.getByRole("button", { name: "Download as PNG" }));
  expect(await preview.findByRole("button", { name: "Image saved" })).toBeInTheDocument();
  expect(saveDiagramPng).toHaveBeenCalledWith("graph TD\nA-->B\n");

  fireEvent.click(preview.getByRole("button", { name: "Image saved" }));
  expect(await preview.findByRole("alert")).toHaveTextContent("Could not save the image");
});

it("stays idle when the save dialog is cancelled", async () => {
  vi.mocked(renderMermaid).mockResolvedValue(DIAGRAM);
  vi.mocked(saveDiagramPng).mockResolvedValueOnce(false);
  render(<MarkdownText text={"```mermaid\ngraph TD\nA-->B\n```"} />);

  fireEvent.click(await screen.findByRole("button", { name: "Download as PNG" }));

  expect(await screen.findByRole("button", { name: "Download as PNG" })).toBeEnabled();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});
