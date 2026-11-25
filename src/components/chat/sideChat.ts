/** A side chat's reply as a Markdown quote for the main chat's draft. */
export const quotedReply = (reply: string) =>
  reply
    .trim()
    .split("\n")
    .map((line) => (line ? `> ${line}` : ">"))
    .join("\n");
