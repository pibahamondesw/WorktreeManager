import { expect, it } from "vitest";
import { droppedPathsText } from "./droppedPaths";

it("escapes shell metacharacters and separates paths with spaces", () => {
  expect(droppedPathsText(["/Users/me/My File (1).png", "/tmp/a&b's.txt", "/w/x-1.ts"])).toBe(
    "/Users/me/My\\ File\\ \\(1\\).png /tmp/a\\&b\\'s.txt /w/x-1.ts "
  );
});

it("keeps non-ASCII characters intact", () => {
  expect(droppedPathsText(["/Users/me/Año 😀.png"])).toBe("/Users/me/Año\\ 😀.png ");
});
