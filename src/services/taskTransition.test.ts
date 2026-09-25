// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";

const suppress = vi.hoisted(() => vi.fn());
vi.mock("./codeEditor", () => ({ editorPresentation: { suppress } }));

import { transitionTaskView } from "./taskTransition";

function renderCard() {
  document.body.innerHTML = `
    <div data-task-card="t1">
      <h3 data-task-part="title">Title</h3>
      <div data-task-part="branch">feat/x</div>
    </div>`;
  return document.querySelector<HTMLElement>("[data-task-card]")!;
}

afterEach(() => {
  delete (document as Partial<Document>).startViewTransition;
  document.body.innerHTML = "";
  suppress.mockReset();
});

it("updates immediately when view transitions are unavailable", () => {
  const update = vi.fn();
  transitionTaskView("t1", update);
  expect(update).toHaveBeenCalledOnce();
  expect(suppress).not.toHaveBeenCalled();
});

it("names the card parts and hides the embedded editor only for the transition", async () => {
  const card = renderCard();
  let finish!: () => void;
  const finished = new Promise<void>((resolve) => (finish = resolve));
  document.startViewTransition = vi.fn((callback) => {
    (callback as () => void)();
    return { finished } as ViewTransition;
  }) as unknown as Document["startViewTransition"];
  const releaseEditor = vi.fn();
  suppress.mockReturnValue(releaseEditor);
  const update = vi.fn();

  transitionTaskView("t1", update);

  expect(update).toHaveBeenCalledOnce();
  expect(card.style.viewTransitionName).toBe("task-shell");
  expect(card.querySelector<HTMLElement>("h3")!.style.viewTransitionName).toBe("task-title");
  expect(releaseEditor).not.toHaveBeenCalled();
  finish();
  await finished;
  await Promise.resolve();
  expect(card.style.viewTransitionName).toBe("");
  expect(releaseEditor).toHaveBeenCalledOnce();
});
