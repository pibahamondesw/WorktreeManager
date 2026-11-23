// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { MarkdownText } from "./ChatItemView";

const parsedTexts = vi.hoisted(() => [] as string[]);

vi.mock("react-markdown", () => ({
  default: ({ children }: { children: string }) => {
    parsedTexts.push(children);
    return <p>{children}</p>;
  },
}));

afterEach(() => {
  cleanup();
  parsedTexts.length = 0;
});

it("parses only the growing last block while text streams in", () => {
  const { rerender } = render(<MarkdownText text={"First\n\nSecond"} />);
  parsedTexts.length = 0;

  rerender(<MarkdownText text={"First\n\nSecond, longer"} />);

  expect(parsedTexts).toEqual(["Second, longer"]);
});

it("parses the whole text when syntax spans blocks", () => {
  render(<MarkdownText text={"See [docs][d]\n\n[d]: https://example.com"} />);

  expect(parsedTexts).toEqual(["See [docs][d]\n\n[d]: https://example.com"]);
});

it("reuses blocks rendered before the chat was remounted", () => {
  const text = "Kept intro\n\nKept body";
  render(<MarkdownText text={text} />).unmount();
  parsedTexts.length = 0;

  render(<MarkdownText text={text} />);

  expect(parsedTexts).toEqual([]);
});

it("caches a streaming block only as last shown", () => {
  const { rerender, unmount } = render(<MarkdownText text={"Stream intro\n\nPar"} />);
  rerender(<MarkdownText text={"Stream intro\n\nPartial"} />);
  rerender(<MarkdownText text={"Stream intro\n\nPartial reply"} />);
  unmount();
  parsedTexts.length = 0;

  render(<MarkdownText text={"Stream intro\n\nPartial"} />);
  cleanup();
  render(<MarkdownText text={"Stream intro\n\nPartial reply"} />);

  expect(parsedTexts).toEqual(["Partial"]);
});
