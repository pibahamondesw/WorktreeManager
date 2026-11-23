import { act } from "@testing-library/react";

const callbacksByElement = new Map<Element, Set<() => void>>();

/** Stands in for `observeResize`; tests fire the callbacks with `resize`. */
export function observeResize(element: Element, callback: () => void) {
  const callbacks = callbacksByElement.get(element) ?? new Set();
  callbacksByElement.set(element, callbacks.add(callback));
  return () => {
    callbacks.delete(callback);
    if (callbacks.size === 0) callbacksByElement.delete(element);
  };
}

export const observedElements = () => [...callbacksByElement.keys()];

export const resize = (elements: Element[] = observedElements()) =>
  act(() =>
    elements.forEach((element) => callbacksByElement.get(element)?.forEach((run) => run()))
  );
