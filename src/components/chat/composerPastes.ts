export const LARGE_PASTE_BYTES = 32 * 1024;

export interface TextAttachment {
  id: string;
  text: string;
  lines: number;
  bytes: number;
}

const byteLength = (text: string) => new TextEncoder().encode(text).length;

export const isLargePaste = (text: string) => byteLength(text) >= LARGE_PASTE_BYTES;

export const textAttachment = (text: string): TextAttachment => ({
  id: crypto.randomUUID(),
  text,
  lines: text.replace(/\n$/, "").split("\n").length,
  bytes: byteLength(text),
});

const formatKilobytes = (bytes: number) => `${Math.round(bytes / 1024).toLocaleString("en-US")} KB`;

export const describeTextAttachment = ({ lines, bytes }: TextAttachment) =>
  `Pasted text · ${lines.toLocaleString("en-US")} lines · ${formatKilobytes(bytes)}`;

function fenceFor(text: string) {
  const longestRun = Math.max(0, ...(text.match(/`+/g) ?? []).map((run) => run.length));
  return "`".repeat(Math.max(3, longestRun + 1));
}

const quotedAttachment = (attachment: TextAttachment) => {
  const fence = fenceFor(attachment.text);
  return `${describeTextAttachment(attachment)}:\n${fence}\n${attachment.text}\n${fence}`;
};

export const withTextAttachments = (message: string, attachments: TextAttachment[]) =>
  [message, ...attachments.map(quotedAttachment)].filter(Boolean).join("\n\n");
