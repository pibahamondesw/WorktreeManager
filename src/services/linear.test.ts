import { describe, it, expect } from "vitest";
import { extractPrsFromAttachments, GqlAttachmentNode } from "./linear";

describe("extractPrsFromAttachments", () => {
  it("returns an empty array for empty attachments", () => {
    expect(extractPrsFromAttachments([])).toEqual([]);
  });

  it("returns an empty array when no attachment has a PR URL", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://example.com/some-page", title: "Docs" },
      { url: "https://github.com/org/repo/issues/42", title: "Issue" },
    ];
    expect(extractPrsFromAttachments(attachments)).toEqual([]);
  });

  it("extracts PR info from a GitHub PR URL", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://github.com/org/repo/pull/123", title: "Fix bug" },
    ];
    expect(extractPrsFromAttachments(attachments)).toEqual([
      {
        url: "https://github.com/org/repo/pull/123",
        title: "Fix bug",
        state: "open",
        number: 123,
        repoSlug: "org/repo",
      },
    ]);
  });

  it("uses a fallback title when attachment has no title", () => {
    const attachments: GqlAttachmentNode[] = [{ url: "https://github.com/org/repo/pull/7" }];
    expect(extractPrsFromAttachments(attachments)[0]?.title).toBe("PR #7");
  });

  it("detects merged state from metadata object", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/10",
        title: "Feature",
        metadata: { status: "Merged" },
      },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.state).toBe("merged");
  });

  it("detects merged state from JSON string metadata", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/10",
        title: "Feature",
        metadata: JSON.stringify({ state: "MERGED" }),
      },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.state).toBe("merged");
  });

  it("detects closed state from metadata", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/5",
        title: "Abandoned",
        metadata: { status: "Closed" },
      },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.state).toBe("closed");
  });

  it("falls back to subtitle for merged detection when metadata has no state", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/20",
        title: "Done",
        subtitle: "Merged by user",
      },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.state).toBe("merged");
  });

  it("falls back to subtitle for closed detection", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/21",
        title: "Nope",
        subtitle: "Closed without merge",
      },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.state).toBe("closed");
  });

  it("returns all PRs when multiple attachments have PR URLs", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://example.com/docs" },
      { url: "https://github.com/org/repo-a/pull/1", title: "First" },
      { url: "https://github.com/org/repo-b/pull/2", title: "Second" },
    ];
    const result = extractPrsFromAttachments(attachments);
    expect(result.map((p) => p.number)).toEqual([1, 2]);
    expect(result.map((p) => p.repoSlug)).toEqual(["org/repo-a", "org/repo-b"]);
  });

  it("dedupes attachments pointing at the same PR", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://github.com/org/repo/pull/1", title: "First" },
      { url: "https://github.com/Org/Repo/pull/1", title: "Duplicate" },
    ];
    expect(extractPrsFromAttachments(attachments)).toHaveLength(1);
  });

  it("skips attachments with no url", () => {
    const attachments: GqlAttachmentNode[] = [
      { title: "No URL" },
      { url: "https://github.com/org/repo/pull/99", title: "Has URL" },
    ];
    expect(extractPrsFromAttachments(attachments)[0]?.number).toBe(99);
  });
});
