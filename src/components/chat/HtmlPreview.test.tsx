// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { MarkdownText } from "./ChatItemView";
import { applyTheme, themes } from "../../themes";
import {
  HTML_PREVIEW_ESCAPE_MESSAGE,
  STATIC_HTML_PREVIEW_CSP,
  htmlPreviewFrameSource,
} from "./htmlPreviewPage";

const platform = vi.hoisted(() => ({ windows: false }));

vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion: vi.fn() }));
vi.mock("../../platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../platform")>()),
  get IS_WINDOWS() {
    return platform.windows;
  },
}));

afterEach(() => {
  cleanup();
  platform.windows = false;
  vi.useRealTimers();
  document.documentElement.removeAttribute("style");
  delete document.documentElement.dataset.appearance;
});

const PAGE = '<h1>Report</h1><script>parent.alert("escaped")</script>';
const PREVIEW = "```html preview\n" + PAGE + "\n```";

const frames = () => screen.queryAllByTitle("HTML preview") as HTMLIFrameElement[];

function framePage(frame: HTMLIFrameElement) {
  const [base, fragment] = frame.getAttribute("src")!.split("#");
  expect(base).toBe("wtm-preview://localhost/");
  const binary = atob(fragment.replace(/-/g, "+").replace(/_/g, "/"));
  return new TextDecoder().decode(Uint8Array.from(binary, (char) => char.charCodeAt(0)));
}

const expectIsolated = (frame: HTMLElement) => {
  expect(frame).toHaveAttribute("sandbox", "allow-scripts");
  expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
  expect(frame).not.toHaveAttribute("srcdoc");
};

it("loads an html preview fence from the preview protocol in a sandbox without same-origin access", () => {
  render(<MarkdownText text={`Here it is:\n\n${PREVIEW}\n\nDone`} />);

  const [frame] = frames();
  expect(frame.tagName).toBe("IFRAME");
  expectIsolated(frame);
  expect(framePage(frame)).toContain(`<body>${PAGE}\n</body>`);
  expect(screen.getByText("Done")).toBeVisible();
});

it("keeps non-ASCII text intact in the page", () => {
  render(<MarkdownText text={"```html preview\n<p>Año 📈</p>\n```"} />);

  expect(framePage(frames()[0])).toContain("<p>Año 📈</p>");
});

it("keeps Windows previews static because WebView2 gives every frame Tauri's invoke key", () => {
  const frame = htmlPreviewFrameSource(PAGE, true);

  expect(frame.sandbox).toBe("");
  expect("src" in frame).toBe(false);
  const page = "srcDoc" in frame ? frame.srcDoc : "";
  expect(
    page.indexOf(`<meta http-equiv="Content-Security-Policy" content="${STATIC_HTML_PREVIEW_CSP}">`)
  ).toBeLessThan(page.indexOf("</head>"));
  expect(page).toContain(`<body>${PAGE}</body>`);
});

it("allows no scripts, connections or remote resources in Windows previews", () => {
  expect(STATIC_HTML_PREVIEW_CSP.split("; ")).toEqual([
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

it("styles the page with the app theme and reloads it on theme changes", () => {
  applyTheme("light");
  render(<MarkdownText text={PREVIEW} />);
  const lightFrame = frames()[0];
  expect(framePage(lightFrame)).toContain(
    `--color-accent: ${themes.find((t) => t.id === "light")!.colors.accent};`
  );
  expect(framePage(lightFrame)).toContain("color-scheme: light;");

  act(() => applyTheme("default"));
  const darkFrame = frames()[0];
  expect(darkFrame).not.toBe(lightFrame);
  expect(framePage(darkFrame)).toContain(`--color-accent: ${themes[0].colors.accent};`);
  expect(framePage(darkFrame)).toContain("color-scheme: dark;");
});

it("opens the page full screen with zoom and closes it with Escape", () => {
  render(<MarkdownText text={PREVIEW} />);
  const opener = screen.getByRole("button", { name: "Open HTML preview full screen" });
  opener.focus();
  fireEvent.click(opener);

  const viewer = screen.getByRole("dialog", { name: "HTML preview" });
  expect(viewer).toHaveFocus();
  const frame = within(viewer).getByTitle("HTML preview") as HTMLIFrameElement;
  expectIsolated(frame);
  expect(frame.getAttribute("src")).toBe(frames()[0].getAttribute("src"));
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

function openFullScreen() {
  render(<MarkdownText text={PREVIEW} />);
  fireEvent.click(screen.getByRole("button", { name: "Open HTML preview full screen" }));
  const viewer = screen.getByRole("dialog", { name: "HTML preview" });
  return { viewer, frame: within(viewer).getByTitle("HTML preview") as HTMLIFrameElement };
}

const postFrom = (frame: HTMLIFrameElement, data: unknown) =>
  act(() => {
    window.dispatchEvent(new MessageEvent("message", { data, source: frame.contentWindow }));
  });

it("closes full screen when its page forwards Escape", () => {
  const { frame } = openFullScreen();

  postFrom(frame, HTML_PREVIEW_ESCAPE_MESSAGE);

  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("ignores Escape messages from other frames and other messages from its page", () => {
  const { frame } = openFullScreen();
  const inlineFrame = frames().find((candidate) => candidate !== frame)!;

  postFrom(inlineFrame, HTML_PREVIEW_ESCAPE_MESSAGE);
  postFrom(frame, "other");
  act(() => {
    window.dispatchEvent(new MessageEvent("message", { data: HTML_PREVIEW_ESCAPE_MESSAGE }));
  });

  expect(screen.getByRole("dialog")).toBeInTheDocument();
});

it("takes focus back from a static page so Escape still closes full screen", async () => {
  platform.windows = true;
  const { viewer, frame } = openFullScreen();
  expect(frame).toHaveAttribute("srcdoc");
  expect(frame).toHaveAttribute("tabindex", "-1");

  frame.focus();
  fireEvent.blur(window);

  await waitFor(() => expect(viewer).toHaveFocus());
});

it("leaves focus in a page with scripts", () => {
  vi.useFakeTimers();
  const { frame } = openFullScreen();
  expect(frame).not.toHaveAttribute("tabindex");

  frame.focus();
  fireEvent.blur(window);
  act(() => {
    vi.runAllTimers();
  });

  expect(frame).toHaveFocus();
});

it("keeps the full screen action outside the page", () => {
  render(<MarkdownText text={PREVIEW} />);

  const bar = screen.getByRole("button", { name: "Open HTML preview full screen" }).parentElement!;
  expect(bar.tagName).toBe("FIGCAPTION");
  expect(bar).not.toContainElement(frames()[0]);
});
