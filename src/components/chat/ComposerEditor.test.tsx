// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { EditorView } from "@codemirror/view";
import { ComposerEditor, ComposerEditorProps } from "./ComposerEditor";

afterEach(cleanup);

const renderEditor = (overrides: Partial<ComposerEditorProps> = {}) => {
  const props: ComposerEditorProps = {
    label: "Message Codex",
    value: "",
    placeholder: "Message Codex",
    disabled: false,
    onChange: vi.fn(),
    onSelect: vi.fn(),
    onKeyDown: vi.fn(),
    onPaste: vi.fn(),
    onBlur: vi.fn(),
    ...overrides,
  };
  const view = render(<ComposerEditor {...props} />);
  const box = screen.getByRole("textbox", { name: "Message Codex" });
  return { props, box, editor: EditorView.findFromDOM(box)!, rerender: view.rerender };
};

it("styles every line of a fenced code block and leaves prose alone", () => {
  const { box } = renderEditor({ value: 'Look:\n```ruby\ndef hola\n  "hello"\nend\n```\nThanks' });

  const codeLines = box.querySelectorAll(".cm-code-line");
  expect([...codeLines].map((line) => line.textContent)).toEqual([
    "```ruby",
    "def hola",
    '  "hello"',
    "end",
    "```",
  ]);
  expect(codeLines[0]).toHaveClass("cm-code-start");
  expect(codeLines[4]).toHaveClass("cm-code-end");
  expect(screen.getByText("Thanks")).not.toHaveClass("cm-code-line");
});

it("reports typed changes but not values set by the composer", () => {
  const { props, editor, rerender } = renderEditor({ value: "draft" });
  expect(props.onChange).not.toHaveBeenCalled();

  editor.dispatch({ changes: { from: 5, insert: "!" }, selection: { anchor: 6 } });
  expect(props.onChange).toHaveBeenLastCalledWith("draft!", 6);

  rerender(<ComposerEditor {...props} value="" />);
  expect(editor.state.doc.toString()).toBe("");
  expect(props.onChange).toHaveBeenCalledTimes(1);
});

it("lets the composer claim keys and inserts a newline on Shift+Enter", () => {
  const onKeyDown = vi.fn((event: KeyboardEvent) => {
    if (event.key === "Enter" && !event.shiftKey) event.preventDefault();
  });
  const { box, editor } = renderEditor({ value: "line", onKeyDown });
  editor.dispatch({ selection: { anchor: 4 } });

  fireEvent.keyDown(box, { key: "Enter" });
  expect(editor.state.doc.toString()).toBe("line");
  expect(onKeyDown).toHaveBeenLastCalledWith(expect.objectContaining({ key: "Enter" }), 4);

  fireEvent.keyDown(box, { key: "Enter", shiftKey: true });
  expect(editor.state.doc.toString()).toBe("line\n");
});

it("does not insert dropped files as text", async () => {
  const { box, editor } = renderEditor({ value: "draft" });
  vi.spyOn(editor, "posAtCoords").mockReturnValue(5);

  fireEvent.drop(box, {
    dataTransfer: { types: ["Files"], files: [new File(["notes"], "notes.txt")] },
  });
  await new Promise((resolve) => setTimeout(resolve, 20));

  expect(editor.state.doc.toString()).toBe("draft");
});

it("is read-only while disabled", () => {
  const { box, editor } = renderEditor({ disabled: true });

  expect(box).toHaveAttribute("aria-disabled", "true");
  expect(box).toHaveAttribute("contenteditable", "false");
  expect(editor.state.readOnly).toBe(true);
});
