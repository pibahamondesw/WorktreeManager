import { ChangeSet, FileChange } from "../../services/chat";
import { DiffLine } from "../../services/semanticDiff";

interface DiffComment {
  id: string;
  text: string;
}

export interface DiffRange {
  start: number;
  end: number;
}

export interface DiffAnchor extends DiffRange {
  diffKey: string;
  path: string;
  location: string;
  quote: string;
}

export interface DiffThread extends DiffAnchor {
  id: string;
  comments: DiffComment[];
}

export const diffKeyOf = (changes: ChangeSet, file: FileChange) =>
  `${changes.before}:${changes.after}:${file.folder}:${file.path}`;

const SIGN: Record<DiffLine["kind"], string> = { added: "+", removed: "-", context: " " };

function numberSpan(numbers: number[]) {
  const first = Math.min(...numbers);
  const last = Math.max(...numbers);
  return first === last ? `${first}` : `${first}-${last}`;
}

function locationOf(path: string, lines: DiffLine[]) {
  const newNumbers = lines.flatMap((line) => line.newNumber ?? []);
  if (newNumbers.length) return `${path}:${numberSpan(newNumbers)}`;
  const oldNumbers = lines.flatMap((line) => line.oldNumber ?? []);
  return `${path} (removed lines ${numberSpan(oldNumbers)})`;
}

export function diffAnchor(
  diffKey: string,
  path: string,
  lines: DiffLine[],
  range: DiffRange
): DiffAnchor {
  const quoted = lines.slice(range.start, range.end + 1);
  return {
    diffKey,
    path,
    start: range.start,
    end: range.end,
    location: locationOf(path, quoted),
    quote: quoted.map((line) => `${SIGN[line.kind]}${line.text}`).join("\n"),
  };
}

const sameAnchor = (thread: DiffThread, anchor: DiffAnchor) =>
  thread.diffKey === anchor.diffKey && thread.start === anchor.start && thread.end === anchor.end;

export function addDiffComment(threads: DiffThread[], anchor: DiffAnchor, text: string) {
  const comment = { id: crypto.randomUUID(), text };
  const existing = threads.find((thread) => sameAnchor(thread, anchor));
  if (!existing) return [...threads, { ...anchor, id: crypto.randomUUID(), comments: [comment] }];
  return threads.map((thread) =>
    thread === existing ? { ...thread, comments: [...thread.comments, comment] } : thread
  );
}

export function replyToDiffThread(threads: DiffThread[], threadId: string, text: string) {
  return threads.map((thread) =>
    thread.id === threadId
      ? { ...thread, comments: [...thread.comments, { id: crypto.randomUUID(), text }] }
      : thread
  );
}

export const removeDiffThread = (threads: DiffThread[], threadId: string) =>
  threads.filter((thread) => thread.id !== threadId);

export function editDiffComment(threads: DiffThread[], commentId: string, text: string) {
  return threads.map((thread) => ({
    ...thread,
    comments: thread.comments.map((comment) =>
      comment.id === commentId ? { ...comment, text } : comment
    ),
  }));
}

export function removeDiffComment(threads: DiffThread[], commentId: string) {
  return threads
    .map((thread) => ({
      ...thread,
      comments: thread.comments.filter((comment) => comment.id !== commentId),
    }))
    .filter((thread) => thread.comments.length > 0);
}

export const diffCommentCount = (threads: DiffThread[]) =>
  threads.reduce((count, thread) => count + thread.comments.length, 0);

function fenceFor(text: string) {
  const longest = Math.max(2, ...(text.match(/`+/g) ?? []).map((run) => run.length));
  return "`".repeat(longest + 1);
}

export function diffReviewMessage(threads: DiffThread[]) {
  const sections = threads.map((thread) => {
    const fence = fenceFor(thread.quote);
    return [
      thread.location,
      `${fence}diff\n${thread.quote}\n${fence}`,
      ...thread.comments.map((comment) => comment.text.trim()),
    ].join("\n\n");
  });
  return ["Review comments on your changes:", ...sections].join("\n\n");
}
