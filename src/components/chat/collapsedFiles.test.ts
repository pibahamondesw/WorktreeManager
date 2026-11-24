import { expect, it } from "vitest";
import { collapsedReason } from "./collapsedFiles";
import { FileChange } from "../../services/chat";

const file = (path: string, changes: Partial<FileChange> = {}): FileChange => ({
  folder: "/wt/a",
  path,
  status: "modified",
  additions: 10,
  deletions: 5,
  binary: false,
  content: "blob",
  ...changes,
});

it.each([
  [file("Cargo.lock"), "Lock file"],
  [file("web/package-lock.json"), "Lock file"],
  [file("src/api.ts", { generated: true }), "Generated file"],
  [file("src/data.ts", { additions: 1000, deletions: 501 }), "Large diff"],
  [file("src/data.ts", { additions: 1000, deletions: 500 }), null],
  [file("docs/lock.md"), null],
])("collapses %o as %s", (change, reason) => {
  expect(collapsedReason(change)).toBe(reason);
});
