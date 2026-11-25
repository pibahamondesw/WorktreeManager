// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ChatItemView, MarkdownText, WorkGroup } from "./ChatItemView";
import { ItemDetailContext } from "./itemDetailContext";
import { renderMermaid } from "./mermaid";
import { saveDiagramPng } from "./diagramExport";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("./mermaid", () => ({ renderMermaid: vi.fn() }));
vi.mock("./diagramExport", () => ({ saveDiagramPng: vi.fn() }));
vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion: vi.fn() }));

beforeEach(() => vi.mocked(openUrl).mockReset().mockResolvedValue(undefined));
afterEach(cleanup);

it.each([
  ["inline double dollar math", "Area $$x^2$$ here", "x^2", false],
  ["inline LaTeX math", String.raw`\(x_i^2\)`, "x_i^2", false],
  ["display dollar math", "$$\n\\frac{1}{2}\n\n+ x\n$$", "\\frac{1}{2}\n\n+ x", true],
  ["display LaTeX math", String.raw`\[\frac{x_i}{y_i}\]`, String.raw`\frac{x_i}{y_i}`, true],
  ["multiline display LaTeX math", "\\[\nx^2 + y^2\n\\]", "x^2 + y^2", true],
  [
    "a LaTeX matrix with a row separator",
    String.raw`\[\begin{matrix} a \\ b \end{matrix}\]`,
    String.raw`\begin{matrix} a \\ b \end{matrix}`,
    true,
  ],
  ["a math fence", "```math\nx^2\n```", "x^2", true],
])("renders %s with MathML", async (_, formula, source, display) => {
  const { container } = render(<MarkdownText text={`Before\n\n${formula}\n\nAfter`} />);
  await waitFor(() =>
    expect(container.querySelector("math annotation")).toHaveTextContent(source, {
      normalizeWhitespace: false,
    })
  );
  expect(container.querySelector(".katex-display") !== null).toBe(display);
  expect(screen.getByText("After")).toBeVisible();
});

it("preserves LaTeX and dollars in code and respects escaped delimiters", () => {
  const inline = String.raw`\(x_i\) and $x$`;
  const escaped = String.raw`\\(x\\) and \$5`;
  const fenced = String.raw`\[a\]`;
  const { container } = render(
    <MarkdownText text={"`" + inline + "`\n\n" + escaped + "\n\n```\n" + fenced + "\n```"} />
  );
  expect(screen.getByText(inline, { selector: "code" })).toBeVisible();
  expect(screen.getByText(fenced, { selector: "code" })).toBeVisible();
  expect(screen.getByText(String.raw`\(x\) and $5`)).toBeVisible();
  expect(container.querySelector(".chat-math-source")).toBeNull();
});

it("shows invalid TeX as source and renders the corrected expression", async () => {
  const { container, rerender } = render(<MarkdownText text={String.raw`$$\frac{1{$$`} />);
  expect(screen.getByText(String.raw`\frac{1{`)).toBeVisible();
  rerender(<MarkdownText text={String.raw`$$\frac{1}{2}$$`} />);
  await waitFor(() =>
    expect(container.querySelector("math annotation")).toHaveTextContent(String.raw`\frac{1}{2}`)
  );

  rerender(<MarkdownText text={String.raw`$$\frac{1{$$`} />);
  expect(screen.getByText(String.raw`\frac{1{`)).toBeVisible();
});

it("keeps single dollar amounts as text", () => {
  const text = "Pay more than $7.000.000: now in transfers of up to $7.000.000 each.";
  const { container } = render(<MarkdownText text={text} />);
  expect(screen.getByText(text)).toBeVisible();
  expect(container.querySelector("code, .chat-math-source")).toBeNull();
});

it("keeps a response readable while a LaTeX delimiter is incomplete", () => {
  render(<MarkdownText text={String.raw`Before \[x^2`} />);
  expect(screen.getByText("Before [x^2")).toBeVisible();
});

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

it("shows a subagent run as a collapsed card with its prompt, steps and report", () => {
  const steps = new Map([
    [
      "a1",
      [
        { id: "s1", kind: "assistant" as const, text: "Checking the parser", parent: "a1" },
        { id: "t1", kind: "command" as const, text: "", title: "cargo test", parent: "a1" },
        { id: "a2", kind: "agent" as const, text: "", title: "Nested look", parent: "a1" },
      ],
    ],
  ]);
  render(
    <ChatItemView
      item={{
        id: "a1",
        kind: "agent",
        title: "Find the bug",
        detail: "Look around the parser",
        text: "Found it in **lexer.rs**",
        status: "completed",
      }}
      steps={steps}
    />
  );

  const card = screen.getByRole("region", { name: "Subagent: Find the bug" });
  const toggle = within(card).getAllByRole("button")[0];
  expect(toggle).toHaveTextContent("3 steps");
  expect(toggle).toHaveTextContent("done");
  expect(screen.queryByText("Checking the parser")).not.toBeInTheDocument();

  fireEvent.click(toggle);
  expect(toggle).toHaveAttribute("aria-expanded", "true");
  expect(within(card).getByText("Look around the parser")).toBeInTheDocument();
  expect(within(card).getByText("Checking the parser")).toBeInTheDocument();
  expect(within(card).getByText("cargo test")).toBeInTheDocument();
  expect(within(card).getByRole("region", { name: "Subagent: Nested look" })).toBeInTheDocument();
  expect(within(card).getByText("lexer.rs")).toBeInTheDocument();
});

it("shows thinking before text arrives and keeps the dropdown open when it completes", () => {
  const item = { id: "r", kind: "reasoning" as const, text: "", status: "inProgress" };
  const { rerender } = render(<ChatItemView item={item} />);
  expect(screen.getByText("Thinking…")).toBeVisible();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(screen.queryByText("Waiting for reasoning…")).not.toBeInTheDocument();

  rerender(<ChatItemView item={{ ...item, text: "Checking the options" }} />);
  const toggle = screen.getByRole("button", { name: "Thinking…" });
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  fireEvent.click(toggle);
  expect(screen.getByText("Checking the options")).toBeVisible();

  rerender(
    <ChatItemView
      item={{ ...item, text: "Done thinking", status: "completed", durationMs: 5200 }}
    />
  );
  expect(screen.getByRole("button", { name: "Thought for 5s" })).toHaveAttribute(
    "aria-expanded",
    "true"
  );
  expect(screen.getByText("Done thinking")).toBeVisible();
});

it("shows historical reasoning without inventing a duration", () => {
  render(<ChatItemView item={{ id: "r", kind: "reasoning", text: "Past reasoning" }} />);
  fireEvent.click(screen.getByRole("button", { name: "Thought" }));
  expect(screen.getByText("Past reasoning")).toBeVisible();
});

it.each(["", " \n "])("shows only the duration when reasoning text is blank: %j", (text) => {
  const { rerender } = render(
    <ChatItemView
      item={{ id: "r", kind: "reasoning", text, status: "completed", durationMs: 100 }}
    />
  );
  expect(screen.getByText("Thought for <1s")).toBeVisible();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(screen.queryByText("No reasoning text provided.")).not.toBeInTheDocument();
  rerender(
    <ChatItemView
      item={{ id: "r", kind: "reasoning", text, status: "stopped", durationMs: 2000 }}
    />
  );
  expect(screen.getByText("Thought for 2s")).toBeVisible();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

it("shows each question the agent asked with its answer", () => {
  render(
    <ChatItemView
      item={{
        id: "q1",
        kind: "question",
        text: "",
        status: "completed",
        questions: [
          { header: "DB", question: "Which DB?", answer: "Postgres" },
          { header: "Cache", question: "Which cache?" },
        ],
      }}
    />
  );

  const card = screen.getByRole("region", { name: "Questions" });
  expect(within(card).getByText("Which DB?")).toBeInTheDocument();
  expect(within(card).getByText("Postgres")).toBeInTheDocument();
  expect(within(card).getByText("Not answered")).toBeInTheDocument();
});

it("shows a question still waiting for its answer", () => {
  render(
    <ChatItemView
      item={{
        id: "q1",
        kind: "question",
        text: "",
        status: "inProgress",
        questions: [{ header: "", question: "Which DB?" }],
      }}
    />
  );

  expect(screen.getByRole("region", { name: "Question" })).toHaveTextContent(
    "Waiting for an answer…"
  );
});

it("shows why a question was rejected", () => {
  render(
    <ChatItemView
      item={{
        id: "q1",
        kind: "question",
        text: "Not now",
        status: "failed",
        questions: [{ header: "", question: "Which DB?" }],
      }}
    />
  );

  const card = screen.getByRole("region", { name: "Question" });
  expect(within(card).getByText("Not answered")).toBeInTheDocument();
  expect(within(card).getByText("Not now")).toBeInTheDocument();
});

it("asks for a command's output while its row is expanded and marks a partial one", () => {
  const expand = vi.fn();
  const item = {
    id: "c1",
    kind: "command" as const,
    text: "",
    title: "make test",
    detailLength: 40960,
  };
  const { rerender } = render(
    <ItemDetailContext.Provider value={expand}>
      <ChatItemView item={item} />
    </ItemDetailContext.Provider>
  );
  expect(expand).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: /make test/ }));
  expect(expand).toHaveBeenLastCalledWith("c1", "tail");
  expect(screen.getByText("Loading output…")).toBeVisible();

  rerender(
    <ItemDetailContext.Provider value={expand}>
      <ChatItemView item={{ ...item, detail: "last lines" }} />
    </ItemDetailContext.Provider>
  );
  expect(screen.getByText("last lines")).toBeVisible();
  expect(screen.getByText("Showing the end of 40.0 KB of output")).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "Show full output" }));
  expect(expand).toHaveBeenLastCalledWith("c1", "full");
  rerender(
    <ItemDetailContext.Provider value={expand}>
      <ChatItemView item={{ ...item, detail: "all lines", detailLength: undefined }} />
    </ItemDetailContext.Provider>
  );
  expect(screen.queryByRole("button", { name: "Show full output" })).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: /make test/ }));
  expect(expand).toHaveBeenLastCalledWith("c1", null);
  fireEvent.click(screen.getByRole("button", { name: /make test/ }));
  expect(expand).toHaveBeenLastCalledWith("c1", "tail");
  expect(expand).toHaveBeenCalledTimes(4);
});

it("keeps a row without output collapsed", () => {
  render(<ChatItemView item={{ id: "c1", kind: "command", text: "", title: "true" }} />);
  expect(screen.getByRole("button", { name: /true/ })).toBeDisabled();
});

it("summarizes a work group and counts its time only while it runs", () => {
  const items = [
    { id: "r1", kind: "tool" as const, text: "", title: "Read" },
    { id: "c1", kind: "command" as const, text: "", title: "make test", status: "failed" },
  ];
  const { rerender } = render(<WorkGroup items={items} runKey="r1" running />);
  const toggle = screen.getByRole("button", { name: "Read 1 file · Ran 1 command · 1 failed" });
  expect(toggle).toHaveTextContent(/0s$/);

  rerender(<WorkGroup items={items} runKey="r1" running={false} />);
  expect(toggle).not.toHaveTextContent(/\ds$/);

  fireEvent.click(toggle);
  expect(screen.getByRole("button", { name: /make test/ }).querySelector("svg")).toBeVisible();
});

it("keeps counting a work group's time when its first step is no longer shown", () => {
  vi.useFakeTimers();
  try {
    const read = { id: "r1", kind: "tool" as const, text: "", title: "Read" };
    const test = { id: "c1", kind: "command" as const, text: "", title: "make test" };
    const { rerender } = render(<WorkGroup items={[read]} runKey="group" running />);
    act(() => {
      vi.advanceTimersByTime(3000);
    });

    rerender(<WorkGroup items={[test]} runKey="group" running />);
    expect(screen.getByRole("button", { name: "Ran 1 command" })).toHaveTextContent(/3s$/);
  } finally {
    vi.useRealTimers();
  }
});

it("counts a subagent's time while it runs", () => {
  const item = { id: "a1", kind: "agent" as const, text: "", title: "Look", status: "inProgress" };
  const { rerender } = render(<ChatItemView item={item} />);
  const card = screen.getByRole("region", { name: "Subagent: Look" });
  expect(card).toHaveTextContent("0s");

  rerender(<ChatItemView item={{ ...item, status: "completed" }} />);
  expect(card).not.toHaveTextContent("0s");
});
