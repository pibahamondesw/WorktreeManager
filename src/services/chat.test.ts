import { describe, expect, it } from "vitest";
import { applyChatEvent, ChatSnapshot } from "./chat";

const base: ChatSnapshot = { generation: 1, status: { kind: "idle" }, items: [], pending: [] };

describe("applyChatEvent", () => {
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
  });

  it("dedupes pending requests and drops them when the session ends", () => {
    const request = { id: "r", title: "Run", kind: "unsupported" as const };
    let s = applyChatEvent(base, { type: "pending", request });
    s = applyChatEvent(s, { type: "pending", request });
    expect(s.pending).toHaveLength(1);
    s = applyChatEvent(s, { type: "status", status: { kind: "busy" } });
    expect(s.pending).toHaveLength(1);
    s = applyChatEvent(s, { type: "status", status: { kind: "failed", message: "x" } });
    expect(s.pending).toEqual([]);
  });
});
