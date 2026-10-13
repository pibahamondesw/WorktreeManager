// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { HistoryDialog } from "./HistoryDialog";

afterEach(cleanup);

const item = {
  id: "user-1",
  kind: "user" as const,
  text: "Change the code",
  checkpoint: "checkpoint-1",
};
const preview = { token: "confirmation", files: ["/repo/a.ts", "/peer/new file.txt"] };

function setup(overrides = {}) {
  const props = {
    item,
    action: "code" as const,
    disabled: false,
    onPreview: vi.fn().mockResolvedValue(preview),
    onApply: vi.fn().mockResolvedValue(undefined),
    onClose: vi.fn(),
    ...overrides,
  };
  render(<HistoryDialog {...props} />);
  return props;
}

describe("HistoryDialog", () => {
  it.each(["code", "both"] as const)(
    "loads the preview immediately and requires explicit confirmation for %s",
    async (action) => {
      const props = setup({ action });
      const files = await screen.findByRole("list", { name: "Files to restore" });
      expect(within(files).getAllByRole("listitem")).toHaveLength(2);
      expect(props.onPreview).toHaveBeenCalledOnce();
      expect(props.onApply).not.toHaveBeenCalled();
      expect(screen.queryByRole("radio")).not.toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Confirm restore" }));
      await waitFor(() => expect(props.onApply).toHaveBeenCalledWith(action, "confirmation"));
      expect(props.onClose).toHaveBeenCalledOnce();
    }
  );

  it("cancels without restoring files", async () => {
    const props = setup();
    await screen.findByRole("list", { name: "Files to restore" });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(props.onApply).not.toHaveBeenCalled();
    expect(props.onClose).toHaveBeenCalledOnce();
  });

  it("shows stale-file errors and requires a new preview before retrying", async () => {
    const onApply = vi.fn().mockRejectedValue("Files changed since the preview");
    const props = setup({ onApply });
    await screen.findByRole("list", { name: "Files to restore" });
    fireEvent.click(screen.getByRole("button", { name: "Confirm restore" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Files changed");
    expect(screen.getByRole("button", { name: "Preview file changes" })).toBeEnabled();
    expect(props.onClose).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Preview file changes" }));
    await screen.findByRole("list", { name: "Files to restore" });
    expect(props.onPreview).toHaveBeenCalledTimes(2);
    expect(onApply).toHaveBeenCalledOnce();
  });

  it("allows retrying a failed initial preview", async () => {
    const onPreview = vi
      .fn()
      .mockRejectedValueOnce("Checkpoint unavailable")
      .mockResolvedValue(preview);
    setup({ onPreview });
    expect(await screen.findByRole("alert")).toHaveTextContent("Checkpoint unavailable");
    fireEvent.click(screen.getByRole("button", { name: "Preview file changes" }));
    await screen.findByRole("list", { name: "Files to restore" });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("keeps restoration disabled while the agent is busy", async () => {
    setup({ disabled: true });
    await screen.findByRole("list", { name: "Files to restore" });
    expect(screen.getByRole("button", { name: "Confirm restore" })).toBeDisabled();
  });
});
