// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { AgentIcon, CheckIcon, FileTypeIcon, PullRequestStateIcon } from "./Icons";

afterEach(cleanup);

const renderedSvg = (element: React.ReactElement) => {
  const svg = render(element).container.querySelector("svg");
  if (!svg) throw new Error("no icon rendered");
  return svg;
};

const glyphName = (svg: SVGSVGElement) =>
  [...svg.classList].find((name) => name.startsWith("lucide-"));

it("sizes library icons in rem so they follow the app zoom", () => {
  const svg = renderedSvg(<CheckIcon size={8} className="text-accent" />);
  expect(svg.getAttribute("width")).toBe("0.5rem");
  expect(svg.getAttribute("height")).toBe("0.5rem");
  expect(svg).toHaveClass("text-accent");
  expect(svg.getAttribute("aria-hidden")).toBe("true");
});

it.each([
  ["src/App.tsx", "lucide-file-code"],
  ["config/settings.YAML", "lucide-file-braces"],
  ["README.md", "lucide-file-text"],
  ["scripts/setup.sh", "lucide-file-terminal"],
  ["assets/logo.png", "lucide-file-image"],
  ["data/report.csv", "lucide-file-spreadsheet"],
  ["release.tar", "lucide-file-archive"],
  ["db/schema.sql", "lucide-file-database"],
  ["demo.mp4", "lucide-file-video-camera"],
  ["sound.wav", "lucide-file-music"],
  ["pnpm-lock.yaml", "lucide-file-lock"],
  ["vendor/Gemfile.lock", "lucide-file-lock"],
  ["Dockerfile", "lucide-file-cog"],
  [".gitignore", "lucide-file-cog"],
  [".env", "lucide-file-key"],
  ["app/.env.local", "lucide-file-key"],
  ["LICENSE", "lucide-file"],
  ["notes.unknown", "lucide-file"],
])("picks the file type icon for %s", (path, glyph) => {
  expect(glyphName(renderedSvg(<FileTypeIcon path={path} />))).toBe(glyph);
});

it("shows each agent's icon and a generic one for agents without their own", () => {
  expect(glyphName(renderedSvg(<AgentIcon agent="claude" />))).toBe("lucide-asterisk");
  cleanup();
  expect(glyphName(renderedSvg(<AgentIcon agent="codex" />))).toBe("lucide-square-terminal");
  cleanup();
  expect(glyphName(renderedSvg(<AgentIcon agent="some-new-agent" />))).toBe("lucide-bot");
});

it.each([
  ["open", "lucide-git-pull-request", "text-success"],
  ["draft", "lucide-git-pull-request-draft", "text-text-muted"],
  ["merged", "lucide-git-merge", "text-accent"],
  ["closed", "lucide-git-pull-request-closed", "text-danger"],
  ["conflicts", "lucide-triangle-exclamation-point", "text-warning"],
] as const)("draws the %s pull request state in its color", (state, glyph, color) => {
  const svg = renderedSvg(<PullRequestStateIcon state={state} className="flex-shrink-0" />);
  expect(glyphName(svg)).toBe(glyph);
  expect(svg).toHaveClass(color, "flex-shrink-0");
  expect(svg.dataset.state).toBe(state);
});
