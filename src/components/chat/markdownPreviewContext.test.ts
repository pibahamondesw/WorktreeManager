// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

const mocks = vi.hoisted(() => ({ chatReadMarkdown: vi.fn() }));
vi.mock("../../services/chat", () => ({ chatReadMarkdown: mocks.chatReadMarkdown }));

import { refreshSavedMarkdown, useMarkdownDocument } from "./markdownPreviewContext";

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

  it("keeps the saved text when an older lookup for that file completes later", async () => {
    const path = "/wt/a/delayed.md";
    const source = { kind: "file" as const, path: "delayed.md" };
    let finishRead!: (value: { path: string; text: string; editable: boolean }) => void;
    mocks.chatReadMarkdown.mockReturnValueOnce(new Promise((resolve) => (finishRead = resolve)));
    const card = renderHook(() => useMarkdownDocument(source, ["/wt/a"]));

    mocks.chatReadMarkdown.mockResolvedValueOnce({ path, text: "Saved", editable: false });
    await act(() => refreshSavedMarkdown(path));
    await act(async () => finishRead({ path, text: "Before", editable: true }));

    expect(card.result.current).toEqual({
      kind: "ready",
      text: "Saved",
      document: { path, text: "Saved", editable: true },
    });
    expect(mocks.chatReadMarkdown).toHaveBeenCalledTimes(2);
  });

  it("publishes only the latest post-save read for a file when reads complete out of order", async () => {
    const path = "/wt/a/two-saves.md";
    const source = { kind: "file" as const, path };
    mocks.chatReadMarkdown.mockResolvedValueOnce(document(path, "Before"));
    const card = renderHook(() => useMarkdownDocument(source, ["/wt/a"]));
    await waitFor(() => expect(card.result.current).toMatchObject({ text: "Before" }));

    let finishFirstRead!: (value: { path: string; text: string }) => void;
    mocks.chatReadMarkdown.mockReturnValueOnce(
      new Promise((resolve) => (finishFirstRead = resolve))
    );
    const firstRefresh = refreshSavedMarkdown(path);
    mocks.chatReadMarkdown.mockResolvedValueOnce(document(path, "Second save"));
    await act(() => refreshSavedMarkdown(path));
    await act(async () => {
      finishFirstRead(document(path, "First save"));
      await firstRefresh;
    });

    expect(card.result.current).toMatchObject({ text: "Second save" });
  });

  it("keeps a document loaded by a lookup started after the post-save read", async () => {
    const path = "/wt/a/reloaded.md";
    const source = { kind: "file" as const, path };
    let finishRefresh!: (value: { path: string; text: string }) => void;
    mocks.chatReadMarkdown.mockReturnValueOnce(new Promise((resolve) => (finishRefresh = resolve)));
    const refresh = refreshSavedMarkdown(path);
    mocks.chatReadMarkdown.mockResolvedValueOnce(document(path, "Later edit"));
    const card = renderHook(() => useMarkdownDocument(source, ["/wt/a"]));
    await waitFor(() => expect(card.result.current).toMatchObject({ text: "Later edit" }));

    await act(async () => {
      finishRefresh(document(path, "Saved"));
      await refresh;
    });

    expect(card.result.current).toMatchObject({ text: "Later edit" });
  });

  it("drops the previous preview when the saved file exceeds the read limit", async () => {
    const path = "/wt/a/large.md";
    const source = { kind: "file" as const, path };
    mocks.chatReadMarkdown.mockResolvedValueOnce(document(path, "Before"));
    const card = renderHook(() => useMarkdownDocument(source, ["/wt/a"]));
    await waitFor(() => expect(card.result.current).toMatchObject({ text: "Before" }));

    const message = `File is too large to preview: ${path}`;
    mocks.chatReadMarkdown.mockRejectedValueOnce(message);
    await act(() => refreshSavedMarkdown(path));

    expect(card.result.current).toEqual({ kind: "error", message });
  });

  it("rejects a post-save read that resolves to a different file", async () => {
    const path = "/wt/a/replaced.md";
    const source = { kind: "file" as const, path };
    mocks.chatReadMarkdown.mockResolvedValueOnce({ path, text: "Before", editable: true });
    const panel = renderHook(() => useMarkdownDocument(source, ["/wt/a"]));
    await waitFor(() => expect(panel.result.current).toMatchObject({ text: "Before" }));

    mocks.chatReadMarkdown.mockResolvedValueOnce({
      path: "/other/replaced.md",
      text: "Other file",
      editable: false,
    });
    await act(() => refreshSavedMarkdown(path));

    expect(panel.result.current).toMatchObject({ kind: "error" });
  });
});
