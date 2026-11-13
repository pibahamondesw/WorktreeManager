import { describe, expect, it } from "vitest";
import {
  documentSummary,
  fileReference,
  markdownPath,
  markdownReferences,
  relativeTo,
  splitFrontmatter,
} from "./markdownDocument";

describe("markdownPath", () => {
  it.each([
    ["docs/plan.md", "docs/plan.md"],
    ["docs/plan.md#L12", "docs/plan.md"],
    ["docs/plan.md:12:3", "docs/plan.md"],
    ["file:///Users/me/notes/a%20b.markdown", "/Users/me/notes/a b.markdown"],
    ["<docs/My Plan.mdx>", "docs/My Plan.mdx"],
    ["~/.claude/plans/x.md", "~/.claude/plans/x.md"],
    ["README.md:3", "README.md"],
  ])("reads %s as %s", (reference, path) => {
    expect(markdownPath(reference)).toBe(path);
  });

  it.each(["https://github.com/a/b/README.md", "mailto:a@b.md", "src/x.ts", undefined])(
    "ignores %s",
    (reference) => {
      expect(markdownPath(reference)).toBeNull();
    }
  );
});

describe("fileReference", () => {
  it.each([
    ["src/x.ts", { path: "src/x.ts" }],
    ["src/x.ts:42", { path: "src/x.ts", line: 42 }],
    ["x.ts:42:7", { path: "x.ts", line: 42 }],
    ["/repo/src/x.ts#L42-L50", { path: "/repo/src/x.ts", line: 42 }],
    ["file:///repo/a%20b.rs", { path: "/repo/a b.rs" }],
  ])("reads %s", (reference, expected) => {
    expect(fileReference(reference)).toEqual(expected);
  });

  it.each(["https://example.com/x.ts", "mailto:a@b.c", "#section", "", undefined])(
    "ignores %s",
    (reference) => {
      expect(fileReference(reference)).toBeNull();
    }
  );
});

describe("markdownReferences", () => {
  it("collects unique Markdown links and code paths outside fenced code", () => {
    const text = [
      "See [the plan](docs/plan.md) and `docs/plan.md:4`.",
      "Also `README.md`, `src/x.ts` and [site](https://x.dev/a.md).",
      "```",
      "cat ignored.md",
      "`fenced.md`",
      "```",
    ].join("\n");
    expect(markdownReferences(text)).toEqual(["docs/plan.md", "README.md"]);
  });
});

describe("splitFrontmatter", () => {
  it("separates leading YAML entries from the body", () => {
    expect(splitFrontmatter('---\ntitle: "Plan"\nstatus: draft\n---\n# Body')).toEqual({
      entries: [
        ["title", "Plan"],
        ["status", "draft"],
      ],
      body: "# Body",
    });
    expect(splitFrontmatter("# No frontmatter\n---")).toEqual({
      entries: [],
      body: "# No frontmatter\n---",
    });
  });
});

describe("documentSummary", () => {
  it("uses the first heading as title and the next five lines as plain text", () => {
    const text = [
      "---",
      "status: draft",
      "---",
      "<!-- hidden -->",
      "# Migration **plan**",
      "",
      "Move [the store](src/store.ts) to `v2`.",
      "",
      "## Steps",
      "- [x] Back up _data_",
      "1. Run it",
      "```ts",
      "const skipped = true;",
      "```",
      "| A | B |",
      "|---|---|",
      "> Quoted",
      "Seventh line",
    ].join("\n");
    expect(documentSummary(text)).toEqual({
      title: "Migration plan",
      lines: ["Move the store to v2.", "Steps", "Back up data", "1. Run it", "A · B"],
    });
  });

  it("has no title when the document has no heading", () => {
    expect(documentSummary("Just text")).toEqual({ title: null, lines: ["Just text"] });
  });
});

describe("relativeTo", () => {
  it("shortens paths inside a task folder", () => {
    expect(relativeTo("/wt/a/docs/plan.md", ["/wt/b", "/wt/a"])).toBe("docs/plan.md");
    expect(relativeTo("/elsewhere/plan.md", ["/wt/a"])).toBe("/elsewhere/plan.md");
  });
});
