// Link definitions (even inside quotes or lists), footnotes, raw HTML, directives and display math can tie distant blocks together.
const DOCUMENT_WIDE_SYNTAX = /^ {0,3}(?:<[A-Za-z!?/]|:::|\$\$)|\]:|\[\^/m;
// A backtick in the rest of the line makes it inline code, not a fence.
const FENCE_PATTERN = /^ {0,3}(`{3,}(?!.*`)|~{3,})/;
// After a blank line, indented content and list items still continue the previous block.
const CONTINUATION_PATTERN = /^(?:[ \t]|(?:[-*+]|\d{1,9}[.)])(?:[ \t]|$))/;

const isFenceClose = (line: string, fence: string, opener: string | undefined) =>
  !!opener && opener[0] === fence[0] && opener.length >= fence.length && line.trim() === opener;

/**
 * Cuts Markdown at blank lines that close every open block, so each piece renders as it would
 * inside the whole text. Returns null when the text uses syntax that spans blocks.
 */
export function splitMarkdownBlocks(text: string): string[] | null {
  if (text.includes("\r") || DOCUMENT_WIDE_SYNTAX.test(text)) return null;
  const blocks: string[] = [];
  let blockStart = 0;
  let lineStart = 0;
  let fence: string | null = null;
  let fenceIndent = 0;
  let afterBlankLine = false;
  for (const line of text.split("\n")) {
    const opener = FENCE_PATTERN.exec(line)?.[1];
    if (fence !== null) {
      // Content left of an indented fence may close the list item holding it, which isn't tracked.
      if (/\S/.test(line.slice(0, fenceIndent))) break;
      if (isFenceClose(line, fence, opener)) fence = null;
    } else if (line.trim() === "") {
      afterBlankLine = true;
    } else {
      if (afterBlankLine && lineStart > blockStart && !CONTINUATION_PATTERN.test(line)) {
        blocks.push(text.slice(blockStart, lineStart));
        blockStart = lineStart;
      }
      afterBlankLine = false;
      fence = opener ?? null;
      if (opener) fenceIndent = line.indexOf(opener);
    }
    lineStart += line.length + 1;
  }
  blocks.push(text.slice(blockStart));
  return blocks;
}
