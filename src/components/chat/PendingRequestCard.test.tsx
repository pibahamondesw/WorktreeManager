// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { PendingRequestCard } from "./PendingRequestCard";
import { PendingRequest } from "../../services/chat";

afterEach(cleanup);

const command = `python3 - <<'PY'\nprint("${"x".repeat(2000)}")\nPY`;
const label = `Approve commands starting with ${command}`;
const request: PendingRequest = {
  id: "long-command",
  title: "Run command",
  detail: command,
  format: "text",
  kind: "approval",
  decisions: [
    { id: "accept", label: "Approve" },
    { id: "acceptForSession", label },
    { id: "decline", label: "Reject" },
  ],
};

it("keeps a multiline approval label readable and sends its original decision", async () => {
  const onRespond = vi.fn().mockResolvedValue(undefined);
  render(<PendingRequestCard request={request} onRespond={onRespond} />);
  const text = screen.getByText(label, { normalizer: (text) => text });
  expect(text).toHaveTextContent("x".repeat(2000));
  expect(text).toHaveClass("whitespace-pre-wrap", "[overflow-wrap:anywhere]");
  const button = text.closest("button")!;
  expect(button).toHaveClass("max-w-full", "min-h-7");
  expect(button).not.toHaveClass("h-7");
  expect(screen.getByText(command, { normalizer: (text) => text })).toHaveClass(
    "[overflow-wrap:anywhere]"
  );
  fireEvent.click(button);
  await waitFor(() => expect(onRespond).toHaveBeenCalledWith({ decision: "acceptForSession" }));
});

it("preserves keyboard approval for long command labels", async () => {
  const onRespond = vi.fn().mockResolvedValue(undefined);
  render(<PendingRequestCard request={request} focused onRespond={onRespond} />);
  const card = screen.getByRole("region", { name: "Run command" });
  expect(card).toHaveFocus();
  fireEvent.keyDown(card, { key: "2" });
  await waitFor(() => expect(onRespond).toHaveBeenCalledWith({ decision: "acceptForSession" }));
});
