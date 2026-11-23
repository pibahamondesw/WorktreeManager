import { flushSync } from "react-dom";

type ResizeCallback = () => void;

const callbacksByElement = new WeakMap<Element, Set<ResizeCallback>>();
let sharedObserver: ResizeObserver | null = null;

function notifyResized(entries: ResizeObserverEntry[]) {
  flushSync(() => {
    for (const entry of entries) {
      callbacksByElement.get(entry.target)?.forEach((callback) => callback());
    }
  });
}

function startObserving(observer: ResizeObserver, element: Element) {
  const callbacks = new Set<ResizeCallback>();
  callbacksByElement.set(element, callbacks);
  observer.observe(element);
  return callbacks;
}

/**
 * Runs `callback` whenever `element` resizes. All callbacks of one observer batch share a single
 * `flushSync`, so state derived from the new size commits in the same frame. Returns the unsubscribe.
 */
export function observeResize(element: Element, callback: ResizeCallback): () => void {
  const observer = (sharedObserver ??= new ResizeObserver(notifyResized));
  const callbacks = callbacksByElement.get(element) ?? startObserving(observer, element);
  callbacks.add(callback);
  return () => {
    if (!callbacks.delete(callback) || callbacks.size > 0) return;
    callbacksByElement.delete(element);
    observer.unobserve(element);
  };
}
