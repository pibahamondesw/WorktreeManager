import { Ref, useEffect, useImperativeHandle, useLayoutEffect, useRef } from "react";
import {
  Annotation,
  Compartment,
  EditorState,
  Prec,
  RangeSetBuilder,
  Transaction,
} from "@codemirror/state";
import {
  Decoration,
  DecorationSet,
  EditorView,
  ViewPlugin,
  ViewUpdate,
  keymap,
  placeholder as placeholderText,
} from "@codemirror/view";
import { history, historyKeymap, standardKeymap } from "@codemirror/commands";
import { HighlightStyle, syntaxHighlighting, syntaxTree } from "@codemirror/language";
import { markdown } from "@codemirror/lang-markdown";
import { languages } from "@codemirror/language-data";
import { tags } from "@lezer/highlight";

export interface ComposerEditorHandle {
  focus: (caret?: number) => void;
  blur: () => void;
}

export interface ComposerEditorProps {
  ref?: Ref<ComposerEditorHandle>;
  label: string;
  value: string;
  placeholder: string;
  disabled: boolean;
  onChange: (value: string, caret: number) => void;
  onSelect: (caret: number) => void;
  onKeyDown: (event: KeyboardEvent, caret: number) => void;
  onPaste: (event: ClipboardEvent) => void;
  onBlur: () => void;
}

type EditorHandlers = Pick<
  ComposerEditorProps,
  "onChange" | "onSelect" | "onKeyDown" | "onPaste" | "onBlur"
>;

const externalValue = Annotation.define<boolean>();
const settings = new Compartment();

const settingsExtension = (label: string, placeholder: string, disabled: boolean) => [
  EditorView.editable.of(!disabled),
  EditorState.readOnly.of(disabled),
  placeholderText(placeholder),
  EditorView.contentAttributes.of({
    "aria-label": label,
    ...(disabled ? { "aria-disabled": "true" } : {}),
  }),
];

const highlightStyle = HighlightStyle.define([
  { tag: tags.keyword, color: "var(--syntax-keyword)" },
  {
    tag: [
      tags.function(tags.variableName),
      tags.function(tags.propertyName),
      tags.definition(tags.variableName),
    ],
    color: "var(--syntax-function)",
  },
  { tag: [tags.string, tags.special(tags.string), tags.regexp], color: "var(--syntax-string)" },
  {
    tag: [tags.number, tags.bool, tags.null, tags.atom, tags.constant(tags.variableName)],
    color: "var(--syntax-literal)",
  },
  { tag: [tags.typeName, tags.className, tags.namespace], color: "var(--syntax-type)" },
  { tag: [tags.propertyName, tags.attributeName], color: "var(--syntax-property)" },
  { tag: tags.comment, color: "var(--syntax-comment)", fontStyle: "italic" },
  {
    tag: [tags.punctuation, tags.operator, tags.processingInstruction, tags.labelName, tags.meta],
    color: "var(--syntax-punctuation)",
  },
  { tag: [tags.link, tags.url], color: "var(--color-accent-hover)" },
  { tag: tags.heading, fontWeight: "600" },
  { tag: tags.strong, fontWeight: "600" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.monospace, fontFamily: "var(--font-code)", fontSize: "0.75rem" },
]);

const theme = EditorView.theme({
  "&": {
    maxHeight: "13.25rem",
    fontSize: "0.875rem",
    color: "var(--color-text-primary)",
    backgroundColor: "transparent",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { overflow: "auto", fontFamily: "inherit", lineHeight: "1.25rem" },
  ".cm-content": {
    minHeight: "3.25rem",
    padding: "0.5rem 0.75rem 0.25rem",
    caretColor: "var(--color-text-primary)",
  },
  ".cm-line": { padding: "0" },
  ".cm-placeholder": { color: "var(--color-text-muted)" },
  ".cm-code-line": {
    padding: "0 0.75rem",
    fontFamily: "var(--font-code)",
    fontSize: "0.75rem",
    backgroundColor: "var(--color-bg-primary)",
  },
  ".cm-code-start": {
    paddingTop: "0.375rem",
    borderTopLeftRadius: "0.5rem",
    borderTopRightRadius: "0.5rem",
  },
  ".cm-code-end": {
    paddingBottom: "0.375rem",
    borderBottomLeftRadius: "0.5rem",
    borderBottomRightRadius: "0.5rem",
  },
});

function codeBlockLines(state: EditorState) {
  const builder = new RangeSetBuilder<Decoration>();
  syntaxTree(state).iterate({
    enter: (node) => {
      if (node.name !== "FencedCode") return;
      const first = state.doc.lineAt(node.from).number;
      const last = state.doc.lineAt(node.to).number;
      for (let number = first; number <= last; number++) {
        const classes = ["cm-code-line"];
        if (number === first) classes.push("cm-code-start");
        if (number === last) classes.push("cm-code-end");
        const line = state.doc.line(number);
        builder.add(line.from, line.from, Decoration.line({ class: classes.join(" ") }));
      }
      return false;
    },
  });
  return builder.finish();
}

const codeBlocks = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = codeBlockLines(view.state);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || syntaxTree(update.startState) !== syntaxTree(update.state))
        this.decorations = codeBlockLines(update.state);
    }
  },
  { decorations: (plugin) => plugin.decorations }
);

const draggedFiles = (event: DragEvent) => !!event.dataTransfer?.types.includes("Files");

function createEditor(parent: HTMLElement, handlers: { current: EditorHandlers }) {
  const caretOf = (view: EditorView) => view.state.selection.main.head;
  return new EditorView({
    parent,
    state: EditorState.create({
      extensions: [
        settings.of([]),
        Prec.highest(
          EditorView.domEventHandlers({
            keydown: (event, view) => {
              handlers.current.onKeyDown(event, caretOf(view));
              return event.defaultPrevented;
            },
            paste: (event) => {
              handlers.current.onPaste(event);
              return event.defaultPrevented;
            },
            drop: (event) => {
              if (!draggedFiles(event)) return false;
              event.preventDefault();
              return true;
            },
            blur: () => handlers.current.onBlur(),
          })
        ),
        keymap.of([...historyKeymap, ...standardKeymap]),
        history(),
        markdown({ codeLanguages: languages, addKeymap: false }),
        syntaxHighlighting(highlightStyle),
        codeBlocks,
        theme,
        EditorView.lineWrapping,
        EditorView.updateListener.of((update) => {
          const caret = update.state.selection.main.head;
          const external = update.transactions.some((tr) => tr.annotation(externalValue));
          if (update.docChanged && !external)
            handlers.current.onChange(update.state.doc.toString(), caret);
          else if (update.selectionSet) handlers.current.onSelect(caret);
        }),
      ],
    }),
  });
}

export function ComposerEditor({
  ref,
  label,
  value,
  placeholder,
  disabled,
  ...handlers
}: ComposerEditorProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const handlersRef = useRef<EditorHandlers>(handlers);

  useLayoutEffect(() => {
    handlersRef.current = handlers;
  });

  useEffect(() => {
    const view = createEditor(hostRef.current!, handlersRef);
    viewRef.current = view;
    return () => {
      view.destroy();
      viewRef.current = null;
    };
  }, []);

  useEffect(() => {
    viewRef.current?.dispatch({
      effects: settings.reconfigure(settingsExtension(label, placeholder, disabled)),
    });
  }, [label, placeholder, disabled]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || view.state.doc.toString() === value) return;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: value },
      selection: { anchor: value.length },
      annotations: [externalValue.of(true), Transaction.addToHistory.of(false)],
    });
  }, [value]);

  useImperativeHandle(
    ref,
    () => ({
      focus: (caret) => {
        const view = viewRef.current;
        if (!view) return;
        view.focus();
        if (caret !== undefined)
          view.dispatch({ selection: { anchor: Math.min(caret, view.state.doc.length) } });
      },
      blur: () => viewRef.current?.contentDOM.blur(),
    }),
    []
  );

  return <div ref={hostRef} className={`select-text ${disabled ? "opacity-50" : ""}`} />;
}
