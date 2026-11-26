// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, renderHook, waitFor } from "@testing-library/react";

const mocks = vi.hoisted(() => ({ chatReadMarkdown: vi.fn() }));
vi.mock("../../services/chat", () => ({ chatReadMarkdown: mocks.chatReadMarkdown }));

import { useMarkdownDocument } from "./markdownPreviewContext";

const document = (path: string, text: string) => ({ path, text });

afterEach(() => {
  cleanup();
  mocks.chatReadMarkdown.mockReset();
});

describe("useMarkdownDocument", () => {
  it("shows a document read before at once when it mounts again, and reads it again", async () => {
    mocks.chatReadMarkdown.mockResolvedValue(document("/wt/a/notes.md", "First"));
    const source = { kind: "file" as const, path: "notes.md" };
    const first = renderHook(() => useMarkdownDocument(source, ["/wt/a"]));
    expect(first.result.current).toEqual({ kind: "loading" });
    await waitFor(() => expect(first.result.current).toMatchObject({ text: "First" }));
    first.unmount();

    mocks.chatReadMarkdown.mockResolvedValue(document("/wt/a/notes.md", "Second"));
    const again = renderHook(() => useMarkdownDocument(source, ["/wt/a"]));
    expect(again.result.current).toMatchObject({ kind: "ready", text: "First" });
    await waitFor(() => expect(again.result.current).toMatchObject({ text: "Second" }));
    expect(mocks.chatReadMarkdown).toHaveBeenCalledTimes(2);
  });

  it("forgets a document it can no longer read", async () => {
    mocks.chatReadMarkdown.mockResolvedValue(document("/wt/a/gone.md", "Before"));
    const source = { kind: "file" as const, path: "gone.md" };
    const read = renderHook(() => useMarkdownDocument(source, ["/wt/a"]));
    await waitFor(() => expect(read.result.current).toMatchObject({ text: "Before" }));
    read.unmount();

    mocks.chatReadMarkdown.mockRejectedValue("File not found: gone.md");
    const first = renderHook(() => useMarkdownDocument(source, ["/wt/a"]));
    await waitFor(() => expect(first.result.current).toMatchObject({ kind: "error" }));
    first.unmount();

    const again = renderHook(() => useMarkdownDocument(source, ["/wt/a"]));
    expect(again.result.current).toEqual({ kind: "loading" });
  });
});
