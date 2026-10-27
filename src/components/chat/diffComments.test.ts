import { describe, expect, it } from "vitest";
import {
  addDiffComment,
  diffAnchor,
  diffCommentCount,
  diffReviewMessage,
  editDiffComment,
  removeDiffComment,
  replyToDiffThread,
} from "./diffComments";
import { DiffLine } from "../../services/semanticDiff";

const line = (kind: DiffLine["kind"], text: string, oldNumber?: number, newNumber?: number) =>
  ({ kind, text, oldNumber, newNumber }) as DiffLine;

const lines = [
  line("context", "keep();", 1, 1),
  line("removed", "old();", 2),
  line("added", "next();", undefined, 2),
  line("added", "more();", undefined, 3),
];

describe("diff comments", () => {
  it("anchors ranges to new line numbers and quotes them as a diff", () => {
    expect(diffAnchor("k", "/wt/a/app.ts", lines, { start: 1, end: 3 })).toEqual({
      diffKey: "k",
      path: "/wt/a/app.ts",
      start: 1,
      end: 3,
      location: "/wt/a/app.ts:2-3",
      quote: "-old();\n+next();\n+more();",
    });
    expect(diffAnchor("k", "/wt/a/app.ts", lines, { start: 1, end: 1 }).location).toBe(
      "/wt/a/app.ts (removed lines 2)"
    );
  });

  it("groups comments on the same range into one thread", () => {
    const anchor = diffAnchor("k", "app.ts", lines, { start: 2, end: 3 });
    let threads = addDiffComment([], anchor, "first");
    threads = addDiffComment(threads, anchor, "second");
    threads = addDiffComment(threads, diffAnchor("k", "app.ts", lines, { start: 0, end: 0 }), "x");
    threads = replyToDiffThread(threads, threads[1].id, "y");

    expect(threads.map((thread) => thread.comments.map((comment) => comment.text))).toEqual([
      ["first", "second"],
      ["x", "y"],
    ]);
    expect(diffCommentCount(threads)).toBe(4);
  });

  it("edits comments and drops threads left empty", () => {
    let threads = addDiffComment([], diffAnchor("k", "app.ts", lines, { start: 0, end: 0 }), "a");
    threads = addDiffComment(threads, diffAnchor("k", "app.ts", lines, { start: 2, end: 2 }), "b");
    const [first, second] = threads.map((thread) => thread.comments[0].id);
    threads = editDiffComment(threads, second, "changed");
    threads = removeDiffComment(threads, first);

    expect(threads).toHaveLength(1);
    expect(threads[0].comments[0].text).toBe("changed");
  });

  it("formats every thread into one message", () => {
    let threads = addDiffComment(
      [],
      diffAnchor("k", "app.ts", lines, { start: 2, end: 3 }),
      "Why?"
    );
    threads = addDiffComment(
      threads,
      diffAnchor("k", "app.ts", lines, { start: 2, end: 3 }),
      "Rename"
    );
    threads = addDiffComment(
      threads,
      diffAnchor("k", "doc.md", [line("added", "```js", undefined, 7)], { start: 0, end: 0 }),
      " Drop this "
    );

    expect(diffReviewMessage(threads)).toBe(
      [
        "Review comments on your changes:",
        "app.ts:2-3",
        "```diff\n+next();\n+more();\n```",
        "Why?",
        "Rename",
        "doc.md:7",
        "````diff\n+```js\n````",
        "Drop this",
      ].join("\n\n")
    );
  });
});
