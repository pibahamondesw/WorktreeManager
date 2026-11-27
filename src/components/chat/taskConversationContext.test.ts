import { expect, it, vi } from "vitest";
import { Task } from "../../types";
import { taskConversations } from "./taskConversationContext";

const task = (id: string, extra: Partial<Task> = {}) =>
  ({ id, branchName: `feature/${id}`, ...extra }) as Task;

it("labels tasks by issue or branch and opens conversations in them", () => {
  const open = vi.fn(() => true);
  const links = taskConversations(
    [task("t1", { linearIssueIdentifier: "WOR-1" }), task("t2")],
    open
  );
  expect(links.label("t1")).toBe("WOR-1");
  expect(links.label("t2")).toBe("feature/t2");
  expect(links.label("gone")).toBeNull();
  expect(links.open({ taskId: "t2", agent: "codex", tab: "a1", conversationId: "th1" })).toBe(true);
  expect(open).toHaveBeenCalledWith({ taskId: "t2", agent: "codex", conversationId: "th1" });
});
