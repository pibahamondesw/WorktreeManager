// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatChanges, FileChange } from "../../services/chat";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../services/syntaxWorker", () => ({ analyzeInWorker: async () => null }));

import { ChangesPanel } from "./ChangesPanel";

const file = (folder: string, path: string): FileChange => ({
  folder,
  path,
  status: "modified",
  additions: 1,
  deletions: 1,
  binary: false,
});

const changeSet = {
  before: "b1",
  after: "a1",
  files: [file("/wt/a", "one.ts"), file("/wt/b", "two.ts"), file("/wt/a", "three.ts")],
};
const changes: ChatChanges = {
  turns: [{ items: ["u"], changes: changeSet, branch: null }],
  conversation: changeSet,
};

function renderPanel(covered = false) {
  render(
    <div data-shortcut-context="task">
      <ChangesPanel
        taskId="t1"
        agent="codex"
        changes={changes}
        prompts={{}}
        scope="conversation"
        threads={[]}
        onThreadsChange={() => {}}
        onSendComments={async () => {}}
        onOpenFile={() => {}}
        covered={covered}
        onClose={() => {}}
      />
      <textarea aria-label="Composer" />
    </div>
  );
  return screen.getByRole("region", { name: "Changes" });
}

const selectedFile = (panel: HTMLElement) =>
  within(panel).getByRole("navigation", { name: "Changed files" }).querySelector("[aria-current]")
    ?.textContent;

const layoutButton = (panel: HTMLElement, name: "unified" | "split") =>
  within(within(panel).getByRole("group", { name: "Diff layout" })).getByRole("button", { name });

beforeEach(() => {
  localStorage.clear();
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue({
    before: { kind: "text", text: "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n" },
    after: { kind: "text", text: "A\nb\nc\nd\ne\nf\ng\nh\ni\nJ\n" },
  });
});
afterEach(cleanup);

describe("ChangesPanel", () => {
  it("remembers the diff layout across panels", () => {
    const panel = renderPanel();
    expect(layoutButton(panel, "unified")).toHaveAttribute("aria-pressed", "true");

    fireEvent.click(layoutButton(panel, "split"));

    expect(localStorage.getItem("worktreemanager.diffLayout")).toBe("split");
    cleanup();
    expect(layoutButton(renderPanel(), "split")).toHaveAttribute("aria-pressed", "true");
  });

  it("walks the changed files in list order and switches the layout from the keyboard", () => {
    const panel = renderPanel();
    expect(selectedFile(panel)).toMatch(/one\.ts/);

    fireEvent.keyDown(panel, { key: "j" });
    expect(selectedFile(panel)).toMatch(/three\.ts/);
    fireEvent.keyDown(panel, { key: "j" });
    expect(selectedFile(panel)).toMatch(/two\.ts/);
    fireEvent.keyDown(panel, { key: "j" });
    expect(selectedFile(panel)).toMatch(/two\.ts/);
    fireEvent.keyDown(panel, { key: "k" });
    expect(selectedFile(panel)).toMatch(/three\.ts/);

    fireEvent.keyDown(panel, { key: "s" });
    expect(layoutButton(panel, "split")).toHaveAttribute("aria-pressed", "true");
  });

  it("leaves the keys to text fields and to other panes", () => {
    const panel = renderPanel();
    const composer = screen.getByLabelText("Composer");

    fireEvent.keyDown(within(panel).getByLabelText("Changes scope"), { key: "j" });
    fireEvent.focus(composer);
    fireEvent.blur(panel, { relatedTarget: composer });
    fireEvent.keyDown(composer, { key: "j" });
    fireEvent.keyDown(document.body, { key: "s" });

    expect(selectedFile(panel)).toMatch(/one\.ts/);
    expect(layoutButton(panel, "unified")).toHaveAttribute("aria-pressed", "true");
  });

  it("leaves the keys alone while the file preview covers the panel", () => {
    const panel = renderPanel(true);

    fireEvent.keyDown(panel, { key: "j" });

    expect(selectedFile(panel)).toMatch(/one\.ts/);
  });

  it("scrolls to the next and previous change, then moves on to the next file", async () => {
    const panel = renderPanel();
    await waitFor(() => expect(panel.querySelectorAll("[data-diff-hunk]")).toHaveLength(2));
    const hunks = [...panel.querySelectorAll<HTMLElement>("[data-diff-hunk]")];
    const scroller = hunks[0].closest<HTMLElement>(".overflow-auto")!;
    let scrollTop = 0;
    Object.defineProperties(scroller, {
      clientHeight: { value: 300 },
      scrollHeight: { value: 900 },
    });
    scroller.scrollBy = vi.fn((options?: ScrollToOptions | number) => {
      scrollTop += typeof options === "object" ? (options.top ?? 0) : 0;
    }) as typeof scroller.scrollBy;
    hunks.forEach((hunk, index) => {
      hunk.getBoundingClientRect = () => ({ top: index * 300 - scrollTop }) as DOMRect;
    });

    fireEvent.keyDown(panel, { key: "n" });
    expect(scrollTop).toBe(300);
    fireEvent.keyDown(panel, { key: "N", shiftKey: true });
    expect(scrollTop).toBe(0);
    fireEvent.keyDown(panel, { key: "n" });
    fireEvent.keyDown(panel, { key: "n" });

    expect(selectedFile(panel)).toMatch(/three\.ts/);
  });

  it("moves on to the next file when the diff cannot scroll to the remaining changes", async () => {
    const panel = renderPanel();
    await waitFor(() => expect(panel.querySelectorAll("[data-diff-hunk]")).toHaveLength(2));
    const hunks = [...panel.querySelectorAll<HTMLElement>("[data-diff-hunk]")];
    hunks.forEach((hunk, index) => {
      hunk.getBoundingClientRect = () => ({ top: 24 + index * 100 }) as DOMRect;
    });

    fireEvent.keyDown(panel, { key: "n" });

    expect(selectedFile(panel)).toMatch(/three\.ts/);
  });
});
