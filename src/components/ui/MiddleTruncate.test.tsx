// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { MiddleTruncate } from "./MiddleTruncate";

afterEach(cleanup);

function parts(value: string) {
  render(<MiddleTruncate value={value} />);
  const root = screen.getByTitle(value);
  return { root, segments: Array.from(root.children).map((child) => child.textContent) };
}

it("keeps the full value as the selectable text and on hover", () => {
  const value = "/Users/someone/Documents/projects/example/repository";
  const { root } = parts(value);
  expect(root).toHaveTextContent(value, { normalizeWhitespace: false });
  expect(root).toHaveClass("select-all");
});

it("keeps the last path segment visible", () => {
  expect(parts("/Users/someone/Documents/projects/vault").segments).toEqual([
    "/Users/someone/Documents/projects",
    "/vault",
  ]);
});

it("ignores a trailing separator when finding the last segment", () => {
  expect(parts("/Users/someone/Documents/vault/").segments).toEqual([
    "/Users/someone/Documents",
    "/vault/",
  ]);
});

it("keeps a fixed tail when the last segment is long", () => {
  expect(parts("someone/feature-with-a-very-long-descriptive-name").segments).toEqual([
    "someone/feature-with-a-very-long-desc",
    "riptive-name",
  ]);
});

it("renders short values whole", () => {
  expect(parts("feat/x").segments).toEqual(["feat/x"]);
});

it("preserves spaces at the split point", () => {
  expect(parts("/Users/someone/Application Support/ app").segments).toEqual([
    "/Users/someone/Application Support",
    "/ app",
  ]);
});
