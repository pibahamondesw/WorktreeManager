// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import type { ComponentProps } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { RemoveWorkspaceModal } from "./RemoveWorkspaceModal";
import { OperationError } from "../../services/operations";
import { Workspace } from "../../types";

afterEach(cleanup);

const workspace: Workspace = { id: "ws", name: "Workspace", repos: [] };

type OnConfirm = ComponentProps<typeof RemoveWorkspaceModal>["onConfirm"];

function renderModal(onConfirm: OnConfirm, onClose = vi.fn()) {
  render(
    <RemoveWorkspaceModal
      open
      onClose={onClose}
      workspace={workspace}
      vault={{ enabled: false, path: null }}
      tasks={[]}
      onConfirm={onConfirm}
    />
  );
  return onClose;
}

it("closes after a clean removal", async () => {
  const onClose = renderModal(
    vi.fn<OnConfirm>().mockResolvedValue({ data: { id: "ws" }, warnings: [] })
  );
  fireEvent.click(screen.getByRole("button", { name: "Remove workspace" }));
  await waitFor(() => expect(onClose).toHaveBeenCalled());
});

it("shows removal warnings before closing", async () => {
  const onClose = renderModal(
    vi.fn<OnConfirm>().mockResolvedValue({
      data: { id: "ws" },
      warnings: [{ stage: "archive_note", message: "Could not archive the note" }],
    })
  );
  fireEvent.click(screen.getByRole("button", { name: "Remove workspace" }));
  expect(await screen.findByText("Could not archive the note")).toBeInTheDocument();
  expect(screen.queryByText(/This workspace has no tasks/)).not.toBeInTheDocument();
  expect(onClose).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Done" }));
  expect(onClose).toHaveBeenCalled();
});

it("shows a failed removal inline and stays open", async () => {
  const onClose = renderModal(
    vi
      .fn<OnConfirm>()
      .mockRejectedValue(new OperationError("sessions_active", "Could not close sessions."))
  );
  fireEvent.click(screen.getByRole("button", { name: "Remove workspace" }));
  expect(await screen.findByText("Could not close sessions.")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Remove" })).not.toBeInTheDocument();
  expect(screen.getAllByRole("button", { name: "Cancel" })).toHaveLength(1);
  expect(onClose).not.toHaveBeenCalled();
});

it("ignores Escape while the removal is running", async () => {
  const onClose = renderModal(vi.fn<OnConfirm>(() => new Promise(() => {})));
  fireEvent.click(screen.getByRole("button", { name: "Remove workspace" }));
  fireEvent.keyDown(document, { key: "Escape" });
  expect(onClose).not.toHaveBeenCalled();
});
