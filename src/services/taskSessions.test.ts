import { describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ editor: vi.fn(), terminal: vi.fn(), chat: vi.fn() }));
vi.mock("./codeEditor", () => ({ editorClose: mocks.editor }));
vi.mock("./terminal", () => ({ terminalClose: mocks.terminal }));
vi.mock("./chat", () => ({ chatClose: mocks.chat }));
import { closeTaskSessions } from "./taskSessions";

describe("task deletion session cleanup", () => {
  it("waits for both sessions and propagates editor shutdown failures", async () => {
    mocks.editor.mockRejectedValueOnce(new Error("still closing"));
    mocks.terminal.mockResolvedValue(undefined);
    mocks.chat.mockResolvedValue(undefined);
    await expect(closeTaskSessions("a")).rejects.toThrow("still closing");
    expect(mocks.terminal).toHaveBeenCalledWith("a");
    expect(mocks.chat).toHaveBeenCalledWith("a");
    mocks.editor.mockResolvedValue(undefined);
    await expect(closeTaskSessions("a")).resolves.toBeUndefined();
  });
});
