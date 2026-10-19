// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { HooksDialog } from "./HooksDialog";
import { HookControls, HookOption } from "../../services/chat";

const hook: HookOption = {
  key: "/user/hooks.json:stop:0:0",
  eventName: "stop",
  sourcePath: "/user/hooks.json",
  currentHash: "sha256:reviewed",
  enabled: true,
  isManaged: false,
  trustStatus: "modified",
  matcher: null,
  handlerType: "command",
  command: "wtm --cli agent event done --agent codex",
  server: null,
  tool: null,
};

const hooks = (items = [hook]): HookControls => ({
  items,
  loading: false,
  error: null,
  warnings: [],
});

afterEach(cleanup);

it("shows the definition and trusts only the selected hash after an explicit click", async () => {
  const onConfigure = vi.fn().mockResolvedValue(undefined);
  render(<HooksDialog hooks={hooks()} onConfigure={onConfigure} onClose={() => undefined} />);
  expect(onConfigure).toHaveBeenCalledExactlyOnceWith({ kind: "hooks" });
  expect(screen.getByText(hook.command!)).toBeVisible();
  expect(screen.getByText(hook.sourcePath)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Trust and enable" }));
  await waitFor(() =>
    expect(onConfigure).toHaveBeenLastCalledWith({
      kind: "hook",
      value: { key: hook.key, hash: hook.currentHash, enabled: true, trust: true },
    })
  );
});

it("keeps managed hooks immutable and does not offer trust for hidden definitions", () => {
  render(
    <HooksDialog
      hooks={hooks([
        { ...hook, key: "managed", isManaged: true },
        {
          ...hook,
          key: "mcp",
          eventName: "postToolUse",
          handlerType: "mcpTool",
          command: null,
          server: "server",
          tool: "tool",
        },
        { ...hook, key: "prompt", eventName: "preToolUse", handlerType: "prompt", command: null },
      ])}
      onConfigure={vi.fn().mockResolvedValue(undefined)}
      onClose={() => undefined}
    />
  );
  expect(
    within(screen.getByRole("region", { name: "stop hook" })).queryByRole("button")
  ).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Trust and enable" })).not.toBeInTheDocument();
  expect(screen.getAllByText("Review this definition in Codex CLI.")).toHaveLength(2);
});

it("disables a hook without granting trust and blocks actions during updates", async () => {
  const onConfigure = vi.fn().mockResolvedValue(undefined);
  const { rerender } = render(
    <HooksDialog hooks={hooks()} onConfigure={onConfigure} onClose={() => undefined} />
  );
  fireEvent.click(screen.getByRole("button", { name: "Disable" }));
  await waitFor(() =>
    expect(onConfigure).toHaveBeenLastCalledWith({
      kind: "hook",
      value: { key: hook.key, hash: hook.currentHash, enabled: false },
    })
  );
  rerender(
    <HooksDialog
      hooks={{ ...hooks(), loading: true }}
      onConfigure={onConfigure}
      onClose={() => undefined}
    />
  );
  expect(screen.getByRole("button", { name: "Trust and enable" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Refresh" })).toBeDisabled();
});

it("shows failed requests and permits refreshing", async () => {
  const onConfigure = vi
    .fn()
    .mockRejectedValueOnce("Codex is unavailable")
    .mockResolvedValue(undefined);
  render(<HooksDialog hooks={undefined} onConfigure={onConfigure} onClose={() => undefined} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Codex is unavailable");
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await waitFor(() => expect(onConfigure).toHaveBeenCalledTimes(2));
});
