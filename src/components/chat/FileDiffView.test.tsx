// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  analyzeInWorker: vi.fn(),
  lineDiff: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../services/syntaxWorker", () => ({ analyzeInWorker: mocks.analyzeInWorker }));
vi.mock("../../services/semanticDiff", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../services/semanticDiff")>();
  mocks.lineDiff.mockImplementation(actual.lineDiff);
  return { ...actual, lineDiff: mocks.lineDiff };
});

import { useEffect, useState } from "react";
import { FileDiffView } from "./FileDiffView";
import { DiffThread } from "./diffComments";
import { FileChange, FileDiff } from "../../services/chat";
import { SourceAnalysis } from "../../services/semanticDiff";
import { analyzeDiffSource } from "../../services/syntaxAnalysis";

const changes = { before: "b", after: "a", files: [] };
const file: FileChange = {
  folder: "/wt/a",
  path: "src/app.ts",
  status: "modified",
  additions: 1,
  deletions: 1,
  binary: false,
  content: "blob",
};

const show = (diff: FileDiff | (() => Promise<never>), props = {}) => {
  mocks.invoke.mockImplementation(() =>
    typeof diff === "function" ? diff() : Promise.resolve(diff)
  );
  const recordThreads = vi.fn<(threads: DiffThread[]) => void>();
  function Harness() {
    const [value, setValue] = useState<DiffThread[]>([]);
    useEffect(() => recordThreads(value), [value]);
    return (
      <FileDiffView
        taskId="t"
        agent="claude"
        changes={changes}
        file={file}
        layout="unified"
        hideFormatting={false}
        threads={value}
        onThreadsChange={setValue}
        {...props}
      />
    );
  }
  const latestThreads = () => recordThreads.mock.lastCall![0];
  return { ...render(<Harness />), latestThreads };
};

const comment = (text: string, button = "Comment") => {
  fireEvent.change(screen.getByRole("textbox", { name: "Comment" }), { target: { value: text } });
  fireEvent.click(screen.getByRole("button", { name: button }));
};

const text = (value: string) => ({ kind: "text" as const, text: value });

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.analyzeInWorker.mockReset().mockImplementation(analyzeDiffSource);
  mocks.lineDiff.mockClear();
});
afterEach(cleanup);

describe("FileDiffView", () => {
  it("renders highlighted unified lines and expands collapsed context", async () => {
    const before = Array.from({ length: 12 }, (_, i) => `const v${i} = ${i};`).join("\n");
    const after = before.replace("const v6 = 6;", "const v6 = 60;");
    const { container } = show({ before: text(before), after: text(after) });

    expect(await screen.findByText("60", { selector: ".diff-changed" })).toBeInTheDocument();
    expect(container.querySelector(".diff-removed .diff-changed")).toHaveTextContent("6");
    expect(container.querySelector(".diff-added .hljs-keyword")).toHaveTextContent("const");
    const gaps = screen.getAllByRole("button", { name: /unchanged lines/ });
    expect(gaps.map((gap) => gap.textContent)).toEqual([
      "⋯ Show 3 unchanged lines",
      "⋯ Show 2 unchanged lines",
    ]);
    fireEvent.click(gaps[0]);
    expect(screen.getByText("v0")).toBeInTheDocument();
  });

  it("opens new-side lines from the split layout", async () => {
    const onOpenLine = vi.fn();
    show({ before: text("a\nb\n"), after: text("a\nc\n") }, { layout: "split", onOpenLine });

    fireEvent.click(await screen.findByRole("button", { name: "Open file at line 2" }));

    expect(onOpenLine).toHaveBeenCalledWith(2);
    expect(screen.getAllByRole("button", { name: /^Open file at line/ })).toHaveLength(2);
  });

  it("shows the diff without highlighting until the worker answers", async () => {
    const answers: (() => Promise<void>)[] = [];
    mocks.analyzeInWorker.mockImplementation(
      (path: string, source: string) =>
        new Promise<SourceAnalysis>((resolve) =>
          answers.push(async () => resolve(await analyzeDiffSource(path, source)))
        )
    );
    const { container } = show({ before: text("const v = 6;"), after: text("const v = 60;") });

    expect(await screen.findByText("60", { selector: ".diff-changed" })).toBeInTheDocument();
    expect(container.querySelector(".hljs-keyword")).toBeNull();
    await Promise.all(answers.map((answer) => answer()));
    await waitFor(() =>
      expect(container.querySelector(".diff-added .hljs-keyword")).toHaveTextContent("const")
    );
  });

  it("keeps the diff without highlighting when the worker fails", async () => {
    mocks.analyzeInWorker.mockResolvedValue(null);
    const { container } = show({ before: text("const v = 6;"), after: text("const v = 60;") });

    expect(await screen.findByText("60", { selector: ".diff-changed" })).toBeInTheDocument();
    await waitFor(() => expect(mocks.analyzeInWorker).toHaveBeenCalledTimes(2));
    expect(container.querySelector(".hljs-keyword")).toBeNull();
  });

  it("highlights the diff without computing the line diff again", async () => {
    const { container } = show({ before: text("const v = 6;"), after: text("const v = 60;") });

    await waitFor(() =>
      expect(container.querySelector(".diff-added .hljs-keyword")).toHaveTextContent("const")
    );
    expect(screen.getByText("60", { selector: ".diff-changed" })).toBeInTheDocument();
    expect(mocks.lineDiff).toHaveBeenCalledTimes(1);
  });

  it("skips syntax analysis when the line diff is too large", async () => {
    mocks.lineDiff.mockReturnValueOnce(null);
    show({ before: text("a"), after: text("b") });

    expect(await screen.findByText("This diff is too large to display.")).toBeInTheDocument();
    expect(mocks.analyzeInWorker).not.toHaveBeenCalled();
  });

  it("pairs removed and added lines side by side and marks formatting changes", async () => {
    const { container } = show(
      { before: text("call(a, b);\nkeep();\n"), after: text("call(\n  a, b);\nkeep();\n") },
      { layout: "split" }
    );

    await screen.findAllByText("keep");
    const rows = container.querySelectorAll(".diff-code > div > div > .flex");
    expect(rows[0].children[0]).toHaveClass("diff-formatting");
    expect(rows[0].children[1]).toHaveClass("diff-formatting");
    expect(rows[1].children[0]).toHaveClass("diff-empty");
    expect(rows[0].children[0]).toHaveAttribute("title", "Formatting-only change");
  });

  it("keeps moved lines on their own split rows", async () => {
    const { container } = show(
      {
        before: text("const moved = compute();\nold();\nkeep();\n"),
        after: text("renamed();\nkeep();\nconst moved = compute();\n"),
      },
      { layout: "split" }
    );

    await screen.findByText("Moved");
    const rows = [...container.querySelectorAll(".diff-code > div > div > .flex")];
    const classes = rows.map((row) =>
      [...row.children].map((cell) =>
        [...cell.classList].find((name) => name.startsWith("diff-") && name !== "diff-row")
      )
    );
    expect(classes).toEqual([
      ["diff-moved", "diff-empty"],
      ["diff-removed", "diff-added"],
      [undefined, undefined],
      ["diff-empty", "diff-moved"],
    ]);
  });

  it("reports formatting-only files when formatting is hidden", async () => {
    show({ before: text("f(a, b)\n"), after: text("f(a,\n  b)\n") }, { hideFormatting: true });

    expect(await screen.findByText("Only formatting changed.")).toBeInTheDocument();
  });

  it("shows images side by side and describes binary files", async () => {
    show({
      before: { kind: "binary", size: 2048 },
      after: { kind: "image", mime: "image/png", data: "AAAA" },
    });

    expect(await screen.findByRole("img", { name: "After" })).toHaveAttribute(
      "src",
      "data:image/png;base64,AAAA"
    );
    expect(screen.getByText("Binary file · 2.0 KB")).toBeInTheDocument();
  });

  it("collects range comments into threads that can be edited and discarded", async () => {
    const { latestThreads } = show({
      before: text("a();\nb();\nc();\n"),
      after: text("a();\nB();\nc();\n"),
    });

    fireEvent.click(await screen.findByRole("button", { name: "Comment on line 1" }));
    fireEvent.click(screen.getByRole("button", { name: "Comment on line 2" }), {
      shiftKey: true,
    });
    comment("Explain this");
    fireEvent.click(screen.getByRole("button", { name: "Add to thread" }));
    comment("And rename it", "Add");

    expect(latestThreads()).toHaveLength(1);
    expect(latestThreads()[0]).toMatchObject({
      location: "/wt/a/src/app.ts:1-2",
      quote: " a();\n-b();\n+B();",
    });
    const thread = screen.getByRole("group", { name: "Comments on /wt/a/src/app.ts:1-2" });
    expect(thread).toHaveTextContent("Explain this");
    expect(thread).toHaveTextContent("And rename it");

    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]);
    comment("Explain this, please", "Save");
    fireEvent.click(screen.getAllByRole("button", { name: "Discard" })[1]);
    expect(latestThreads()[0].comments.map((entry) => entry.text)).toEqual([
      "Explain this, please",
    ]);
  });

  it("anchors comments on formatting lines shown as context", async () => {
    const { latestThreads } = show(
      { before: text("a();\n"), after: text("a();\n\nb();\n") },
      { hideFormatting: true }
    );

    fireEvent.click(await screen.findByRole("button", { name: "Comment on line 2" }));
    expect(screen.getAllByRole("textbox", { name: "Comment" })).toHaveLength(1);
    comment("Remove the blank line");

    expect(latestThreads()[0]).toMatchObject({ location: "/wt/a/src/app.ts:2", quote: "+" });
  });

  it("shows load errors", async () => {
    show(() => Promise.reject("Checkpoint data is damaged"));

    expect(await screen.findByRole("alert")).toHaveTextContent("Checkpoint data is damaged");
  });
});
