import { describe, it, expect } from "vitest";
import { extractPrFromAttachments, GqlAttachmentNode } from "./linear";

describe("extractPrFromAttachments", () => {
  it("returns null for empty attachments", () => {
    expect(extractPrFromAttachments([])).toBeNull();
  });

  it("returns null when no attachment has a PR URL", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://example.com/some-page", title: "Docs" },
      { url: "https://github.com/org/repo/issues/42", title: "Issue" },
    ];
    expect(extractPrFromAttachments(attachments)).toBeNull();
  });

  it("extracts PR info from a GitHub PR URL", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://github.com/org/repo/pull/123", title: "Fix bug" },
    ];
    const result = extractPrFromAttachments(attachments);
    expect(result).toEqual({
      url: "https://github.com/org/repo/pull/123",
      title: "Fix bug",
      state: "open",
      number: 123,
    });
  });

  it("uses a fallback title when attachment has no title", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://github.com/org/repo/pull/7" },
    ];
    expect(extractPrFromAttachments(attachments)?.title).toBe("PR #7");
  });

  it("detects merged state from metadata object", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/10",
        title: "Feature",
        metadata: { status: "Merged" },
      },
    ];
    expect(extractPrFromAttachments(attachments)?.state).toBe("merged");
  });

  it("detects merged state from JSON string metadata", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/10",
        title: "Feature",
        metadata: JSON.stringify({ state: "MERGED" }),
      },
    ];
    expect(extractPrFromAttachments(attachments)?.state).toBe("merged");
  });

  it("detects closed state from metadata", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/5",
        title: "Abandoned",
        metadata: { status: "Closed" },
      },
    ];
    expect(extractPrFromAttachments(attachments)?.state).toBe("closed");
  });

  it("falls back to subtitle for merged detection when metadata has no state", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/20",
        title: "Done",
        subtitle: "Merged by user",
      },
    ];
    expect(extractPrFromAttachments(attachments)?.state).toBe("merged");
  });

  it("falls back to subtitle for closed detection", () => {
    const attachments: GqlAttachmentNode[] = [
      {
        url: "https://github.com/org/repo/pull/21",
        title: "Nope",
        subtitle: "Closed without merge",
      },
    ];
    expect(extractPrFromAttachments(attachments)?.state).toBe("closed");
  });

  it("returns the first PR when multiple attachments have PR URLs", () => {
    const attachments: GqlAttachmentNode[] = [
      { url: "https://example.com/docs" },
      { url: "https://github.com/org/repo/pull/1", title: "First" },
      { url: "https://github.com/org/repo/pull/2", title: "Second" },
    ];
    expect(extractPrFromAttachments(attachments)?.number).toBe(1);
  });

  it("skips attachments with no url", () => {
    const attachments: GqlAttachmentNode[] = [
      { title: "No URL" },
      { url: "https://github.com/org/repo/pull/99", title: "Has URL" },
    ];
    expect(extractPrFromAttachments(attachments)?.number).toBe(99);
  });
});
