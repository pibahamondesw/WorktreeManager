import { flushSync } from "react-dom";
import { editorPresentation } from "./codeEditor";

export const TASK_TRANSITION_PARTS = ["shell", "identifier", "status", "title", "branch"] as const;
export type TaskTransitionPart = (typeof TASK_TRANSITION_PARTS)[number];

export const taskTransitionName = (part: TaskTransitionPart) => `task-${part}`;

function canAnimate() {
  return (
    typeof document.startViewTransition === "function" &&
    !window.matchMedia?.("(prefers-reduced-motion: reduce)").matches
  );
}

function nameCardParts(taskId: string) {
  const card = [...document.querySelectorAll<HTMLElement>("[data-task-card]")].find(
    (element) => element.dataset.taskCard === taskId
  );
  if (!card) return () => undefined;
  const parts = [card, ...card.querySelectorAll<HTMLElement>("[data-task-part]")];
  for (const element of parts) {
    const part = (element.dataset.taskPart ?? "shell") as TaskTransitionPart;
    element.style.viewTransitionName = taskTransitionName(part);
  }
  return () => parts.forEach((element) => (element.style.viewTransitionName = ""));
}

/** Morphs the task's card into the task header (or back) while `update` swaps the views. */
export function transitionTaskView(taskId: string, update: () => void) {
  if (!canAnimate()) {
    update();
    return;
  }
  const unnameCard = nameCardParts(taskId);
  const releaseEditor = editorPresentation.suppress();
  const transition = document.startViewTransition(() => flushSync(update));
  void transition.finished.finally(() => {
    unnameCard();
    releaseEditor();
  });
}
