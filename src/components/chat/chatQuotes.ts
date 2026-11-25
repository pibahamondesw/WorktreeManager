export interface ChatQuote {
  id: string;
  /** The reply the text comes from, which is also its timeline row. */
  itemId: string;
  text: string;
  /** The other agent that wrote the reply, when it came from that agent's conversation. */
  author?: string;
}

const EXCERPT_LENGTH = 80;

export const chatQuote = (itemId: string, text: string, author?: string): ChatQuote => ({
  id: crypto.randomUUID(),
  itemId,
  text,
  author,
});

export function quoteExcerpt({ text }: ChatQuote) {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > EXCERPT_LENGTH ? `${flat.slice(0, EXCERPT_LENGTH - 1)}…` : flat;
}

const quotedReply = ({ text, author }: ChatQuote) =>
  [
    author ? `From the earlier ${author} reply:` : "From your earlier reply:",
    ...text.split("\n").map((line) => `> ${line}`.trimEnd()),
  ].join("\n");

export const withQuotes = (message: string, quotes: ChatQuote[]) =>
  [...quotes.map(quotedReply), message].filter(Boolean).join("\n\n");
