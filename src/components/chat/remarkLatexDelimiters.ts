import type { Extension as FromMarkdownExtension } from "mdast-util-from-markdown";
import type { Extension as MicromarkExtension, State, Tokenizer } from "micromark-util-types";
import type { Processor } from "unified";

// These are the extension slots consumed by react-markdown's remark parser.
declare module "unified" {
  interface Data {
    micromarkExtensions?: MicromarkExtension[];
    fromMarkdownExtensions?: Array<FromMarkdownExtension | FromMarkdownExtension[]>;
  }
}

declare module "micromark-util-types" {
  interface TokenTypeMap {
    latexMath: "latexMath";
    latexMathData: "latexMathData";
    latexMathLineEnding: "latexMathLineEnding";
  }
}

const BACKSLASH = 92;
const OPEN_PAREN = 40;
const CLOSE_PAREN = 41;
const OPEN_BRACKET = 91;
const CLOSE_BRACKET = 93;

// Match before CommonMark turns \[ and \( into literal punctuation. A text
// construct leaves code spans, fenced code and link destinations untouched.
const tokenize: Tokenizer = (effects, ok, nok) => {
  let closing: number;
  // Micromark uses -5 through -3 for line endings. Each needs its own
  // token so the text tokenizer can map continued lines back to the document.
  const betweenLines: State = (code) => {
    if (code === null) return nok(code);
    if (code <= -3) {
      effects.enter("latexMathLineEnding");
      effects.consume(code);
      effects.exit("latexMathLineEnding");
      return betweenLines;
    }
    effects.enter("latexMathData");
    return body(code);
  };
  const body: State = (code) => {
    if (code === null) return nok(code);
    if (code <= -3) {
      effects.exit("latexMathData");
      return betweenLines(code);
    }
    effects.consume(code);
    return code === BACKSLASH ? escape : body;
  };
  const escape: State = (code) => {
    if (code === closing) {
      effects.consume(code);
      effects.exit("latexMathData");
      effects.exit("latexMath");
      return ok;
    }
    if (code === BACKSLASH) {
      effects.consume(code);
      return body;
    }
    return body(code);
  };
  return (code) => {
    effects.enter("latexMath");
    effects.enter("latexMathData");
    effects.consume(code);
    return (opener) => {
      if (opener !== OPEN_PAREN && opener !== OPEN_BRACKET) return nok(opener);
      closing = opener === OPEN_PAREN ? CLOSE_PAREN : CLOSE_BRACKET;
      effects.consume(opener);
      return body;
    };
  };
};

const fromMarkdown: FromMarkdownExtension = {
  exit: {
    latexMath(token) {
      const raw = this.sliceSerialize(token);
      const display = raw.startsWith("\\[");
      const value = raw.slice(2, -2).trim();
      this.enter(
        {
          type: "inlineMath",
          value,
          data: {
            hName: "code",
            hProperties: { className: [display ? "math-display" : "math-inline"] },
            hChildren: [{ type: "text", value }],
          },
        },
        token
      );
      this.exit(token);
    },
  },
};

export function remarkLatexDelimiters(this: Processor) {
  const data = this.data();
  (data.micromarkExtensions ??= []).push({ text: { [BACKSLASH]: { tokenize } } });
  (data.fromMarkdownExtensions ??= []).push(fromMarkdown);
}
