import { describe, expect, it } from "vitest";
import { applyChatEvent, ChatItem, ChatSnapshot, EMPTY_CONTROLS, limitResetsAt } from "./chat";

const base: ChatSnapshot = {
  generation: 1,
  status: { kind: "idle" },
  items: [],
  pending: [],
  controls: EMPTY_CONTROLS,
};

describe("applyChatEvent", () => {
  it("places earlier items before the loaded ones until the items are replaced", () => {
    let s = applyChatEvent(
      { ...base, items: [{ id: "b", kind: "user", text: "" }], earlier: 3 },
      { type: "earlier", items: [{ id: "a", kind: "user", text: "" }], earlier: 2 }
    );
    expect(s.items.map((item) => item.id)).toEqual(["a", "b"]);
    expect(s.earlier).toBe(2);
    s = applyChatEvent(s, { type: "items", items: [] });
    expect(s.earlier).toBe(0);
  });

  it("streams deltas into an upserted item and lets the final item replace it", () => {
    let s = applyChatEvent(base, {
      type: "upsert",
      item: { id: "a", kind: "assistant", text: "" },
    });
    s = applyChatEvent(s, { type: "delta", id: "a", field: "text", delta: "Hi" });
    s = applyChatEvent(s, { type: "delta", id: "missing", field: "text", delta: "x" });
    s = applyChatEvent(s, { type: "delta", id: "a", field: "detail", delta: "out" });
    expect(s.items).toEqual([{ id: "a", kind: "assistant", text: "Hi", detail: "out" }]);
    s = applyChatEvent(s, { type: "upsert", item: { id: "a", kind: "assistant", text: "Hi!" } });
    expect(s.items).toHaveLength(1);
    expect(s.items[0].text).toBe("Hi!");
    s = applyChatEvent(s, { type: "remove", id: "a" });
    expect(s.items).toEqual([]);
  });

  it("dedupes pending requests and drops them when the session ends", () => {
    const request = {
      id: "r",
      title: "Run",
      format: "text" as const,
      kind: "unsupported" as const,
    };
    let s = applyChatEvent(base, { type: "pending", request });
    s = applyChatEvent(s, { type: "pending", request });
    expect(s.pending).toHaveLength(1);
    s = applyChatEvent(s, { type: "status", status: { kind: "busy" } });
    expect(s.pending).toHaveLength(1);
    s = applyChatEvent(s, { type: "status", status: { kind: "failed", message: "x" } });
    expect(s.pending).toEqual([]);
  });
});

describe("limitResetsAt", () => {
  const limit: ChatItem = { id: "e1", kind: "error", text: "Limit", limitResetsAt: 100 };
  const prompt: ChatItem = { id: "u1", kind: "user", text: "Hi" };

  it("reads the renewal of a usage limit error until the next prompt", () => {
    expect(limitResetsAt([prompt, limit, { id: "a1", kind: "assistant", text: "" }])).toBe(100);
    expect(limitResetsAt([limit, prompt])).toBeNull();
    expect(limitResetsAt([prompt, { ...limit, origin: "codex" }])).toBeNull();
    expect(limitResetsAt([prompt, { ...limit, limitResetsAt: undefined }])).toBeNull();
    expect(limitResetsAt([])).toBeNull();
  });
});
