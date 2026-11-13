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
