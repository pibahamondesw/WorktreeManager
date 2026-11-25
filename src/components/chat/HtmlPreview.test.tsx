// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { MarkdownText } from "./ChatItemView";
import { applyTheme, themes } from "../../themes";
import { HTML_PREVIEW_CSP } from "./htmlPreviewPage";

vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion: vi.fn() }));

afterEach(() => {
  cleanup();
  document.documentElement.removeAttribute("style");
  delete document.documentElement.dataset.appearance;
});

const PAGE = '<h1>Report</h1><script>parent.alert("escaped")</script>';
const PREVIEW = "```html preview\n" + PAGE + "\n```";

const frames = () => screen.queryAllByTitle("HTML preview") as HTMLIFrameElement[];

it("renders an html preview fence in a sandbox without scripts or same-origin access", () => {
  render(<MarkdownText text={`Here it is:\n\n${PREVIEW}\n\nDone`} />);

  const [frame] = frames();
  expect(frame.tagName).toBe("IFRAME");
  expect(frame).toHaveAttribute("sandbox", "");
  expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
  expect(frame).not.toHaveAttribute("src");
  const page = frame.getAttribute("srcdoc")!;
  expect(
    page.indexOf(`<meta http-equiv="Content-Security-Policy" content="${HTML_PREVIEW_CSP}">`)
  ).toBeLessThan(page.indexOf("</head>"));
  expect(page).toContain(`<body>${PAGE}\n</body>`);
  expect(screen.getByText("Done")).toBeVisible();
});

it("allows no scripts, connections or remote resources in the page", () => {
  expect(HTML_PREVIEW_CSP.split("; ")).toEqual([
    "default-src 'none'",
    "style-src 'unsafe-inline'",
    "img-src data:",
    "font-src data:",
    "media-src data:",
  ]);
});

it("keeps plain html fences as code", () => {
  render(<MarkdownText text={"```html\n<h1>Report</h1>\n```"} />);

  expect(frames()).toHaveLength(0);
  expect(screen.getByText("html")).toBeInTheDocument();
});

it("shows the source until the fence closes so streaming doesn't reload the page", () => {
  const { rerender } = render(<MarkdownText text={"```html preview\n<h1>Rep"} />);
  expect(frames()).toHaveLength(0);
  expect(screen.getByText(/Rep/)).toBeInTheDocument();

  rerender(<MarkdownText text={"```html preview\n<h1>Report</h1>\n```\n"} />);
  expect(frames()).toHaveLength(1);
});

it.each([
  ["a blockquote", "> ```html preview\n> <h1>Report</h1>\n> ```", 1],
  ["a shorter closing run", "````html preview\n<pre>\n```", 0],
  ["a different closing character", "```html preview\n<h1>Report</h1>\n~~~", 0],
  ["a longer closing run", "```html preview\n<h1>Report</h1>\n`````", 1],
])("matches closing fences in %s like CommonMark", (_, text, count) => {
  render(<MarkdownText text={text} />);

  expect(frames()).toHaveLength(count);
});

it("renders a closed tilde fence inside a list", () => {
  render(<MarkdownText text={"- Chart:\n\n  ~~~html preview\n  <svg></svg>\n  ~~~"} />);

  expect(frames()).toHaveLength(1);
});

it("styles the page with the app theme and follows theme changes", () => {
  applyTheme("light");
  render(<MarkdownText text={PREVIEW} />);
  const page = () => frames()[0].getAttribute("srcdoc")!;
  expect(page()).toContain(
    `--color-accent: ${themes.find((t) => t.id === "light")!.colors.accent};`
  );
  expect(page()).toContain("color-scheme: light;");

  act(() => applyTheme("default"));
  expect(page()).toContain(`--color-accent: ${themes[0].colors.accent};`);
  expect(page()).toContain("color-scheme: dark;");
});

it("opens the page full screen with zoom and closes it with Escape", () => {
  render(<MarkdownText text={PREVIEW} />);
  const opener = screen.getByRole("button", { name: "Open HTML preview full screen" });
  opener.focus();
  fireEvent.click(opener);

  const viewer = screen.getByRole("dialog", { name: "HTML preview" });
  expect(viewer).toHaveFocus();
  const frame = within(viewer).getByTitle("HTML preview");
  expect(frame).toHaveAttribute("sandbox", "");
  const status = within(viewer).getByRole("status");

  fireEvent.click(within(viewer).getByRole("button", { name: "Zoom in" }));
  expect(status).toHaveTextContent("125%");
  expect(frame.parentElement!.style.transform).toBe("scale(1.25)");
  expect(frame.parentElement!.style.width).toBe("80%");
  fireEvent.keyDown(viewer, { key: "+" });
  fireEvent.keyDown(viewer, { key: "=" });
  fireEvent.keyDown(viewer, { key: "+" });
  fireEvent.keyDown(viewer, { key: "+" });
  expect(status).toHaveTextContent("200%");
  expect(within(viewer).getByRole("button", { name: "Zoom in" })).toBeDisabled();

  fireEvent.keyDown(viewer, { key: "0" });
  expect(status).toHaveTextContent("100%");
  fireEvent.click(within(viewer).getByRole("button", { name: "Zoom out" }));
  fireEvent.keyDown(viewer, { key: "-" });
  expect(status).toHaveTextContent("50%");
  expect(within(viewer).getByRole("button", { name: "Zoom out" })).toBeDisabled();
  fireEvent.click(within(viewer).getByRole("button", { name: "Zoom 50%. Reset zoom" }));
  expect(status).toHaveTextContent("100%");

  fireEvent.keyDown(viewer, { key: "+", metaKey: true });
  fireEvent.keyDown(viewer, { key: "x" });
  expect(status).toHaveTextContent("100%");

  fireEvent.keyDown(viewer, { key: "Escape" });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(opener).toHaveFocus();
});

it("closes full screen from its close button", () => {
  render(<MarkdownText text={PREVIEW} />);
  fireEvent.click(screen.getByRole("button", { name: "Open HTML preview full screen" }));

  fireEvent.click(screen.getByRole("button", { name: "Close HTML preview full screen" }));

  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
