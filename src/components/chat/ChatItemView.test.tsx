// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { openUrl } from "@tauri-apps/plugin-opener";
import { MarkdownText } from "./ChatItemView";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

beforeEach(() => vi.mocked(openUrl).mockReset().mockResolvedValue(undefined));
afterEach(cleanup);

it.each([
  [
    "[Documentation](https://example.com/docs?tab=api#usage)",
    "Documentation",
    "https://example.com/docs?tab=api#usage",
  ],
  ["https://example.com", "https://example.com", "https://example.com"],
])("opens Markdown links externally: %s", (text, label, url) => {
  render(<MarkdownText text={text} />);

  expect(fireEvent.click(screen.getByRole("link", { name: label }))).toBe(false);
  expect(openUrl).toHaveBeenCalledExactlyOnceWith(url);
});

it.each([{ ctrlKey: true }, { metaKey: true }, { shiftKey: true }])(
  "prevents webview navigation for modified clicks: %s",
  (modifiers) => {
    render(<MarkdownText text="[Documentation](https://example.com)" />);

    expect(fireEvent.click(screen.getByRole("link"), modifiers)).toBe(false);
    expect(openUrl).toHaveBeenCalledExactlyOnceWith("https://example.com");
  }
);

it("opens middle clicks externally and ignores right clicks", () => {
  render(<MarkdownText text="[Documentation](https://example.com)" />);
  const link = screen.getByRole("link");

  fireEvent(link, new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 2 }));
  expect(openUrl).not.toHaveBeenCalled();
  expect(
    fireEvent(link, new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }))
  ).toBe(false);
  expect(openUrl).toHaveBeenCalledExactlyOnceWith("https://example.com");
});

it("keeps the conversation visible when opening fails and allows retrying", async () => {
  vi.mocked(openUrl).mockRejectedValueOnce(new Error("Browser unavailable"));
  render(<MarkdownText text="Conversation with [Documentation](https://example.com)" />);
  const link = screen.getByRole("link");

  expect(fireEvent.click(link)).toBe(false);
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not open the link");
  expect(screen.getByText(/Conversation with/)).toBeVisible();

  expect(fireEvent.click(link)).toBe(false);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(openUrl).toHaveBeenCalledTimes(2);
});

it("does not open unsafe URLs stripped by Markdown", () => {
  render(<MarkdownText text="[Unsafe](javascript:alert%281%29)" />);

  expect(fireEvent.click(screen.getByText("Unsafe"))).toBe(false);
  expect(openUrl).not.toHaveBeenCalled();
});
