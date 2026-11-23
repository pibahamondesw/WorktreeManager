import { expect, it } from "vitest";
import { LruCache } from "./lruCache";

it("returns stored values and nothing for unknown keys", () => {
  const cache = new LruCache<string>(2, 100);
  cache.set("a", "first", 1);

  expect(cache.get("a")).toBe("first");
  expect(cache.get("b")).toBeUndefined();
});

it("evicts the least recently used entry past the entry limit", () => {
  const cache = new LruCache<string>(2, 100);
  cache.set("a", "first", 1);
  cache.set("b", "second", 1);
  cache.get("a");

  cache.set("c", "third", 1);

  expect(cache.get("a")).toBe("first");
  expect(cache.get("b")).toBeUndefined();
  expect(cache.get("c")).toBe("third");
});

it("evicts the oldest entries until the total size fits", () => {
  const cache = new LruCache<string>(10, 10);
  cache.set("a", "first", 4);
  cache.set("b", "second", 4);

  cache.set("c", "third", 6);

  expect(cache.get("a")).toBeUndefined();
  expect(cache.get("b")).toBe("second");
  expect(cache.get("c")).toBe("third");
});

it("skips entries larger than the whole budget", () => {
  const cache = new LruCache<string>(10, 10);
  cache.set("a", "first", 4);

  cache.set("b", "huge", 11);

  expect(cache.get("a")).toBe("first");
  expect(cache.get("b")).toBeUndefined();
});

it("replaces an entry and frees its previous size", () => {
  const cache = new LruCache<string>(10, 10);
  cache.set("a", "first", 8);
  cache.set("a", "replaced", 2);

  cache.set("b", "second", 8);

  expect(cache.get("a")).toBe("replaced");
  expect(cache.get("b")).toBe("second");
});
