// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { AddWorkspaceModal } from "./AddWorkspaceModal";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: () => Promise.resolve("/Users/me/") }));

const openMock = vi.mocked(openDialog);

const NAME_PLACEHOLDER = "e.g. Payments";

async function renderModal() {
  const onAdd = vi.fn();
  render(<AddWorkspaceModal open onClose={vi.fn()} onAdd={onAdd} />);
  // Flush the homeDir() effect so repos get a default worktree base path.
  await act(async () => {});
  return onAdd;
}

async function addRepo(path: string) {
  openMock.mockResolvedValueOnce(path);
  await act(async () => {
    fireEvent.click(screen.getByText("+ Add repo"));
  });
}

const nameField = () => screen.getByPlaceholderText(NAME_PLACEHOLDER);
const submit = () => fireEvent.click(screen.getByText("Add Workspace", { selector: "button" }));

beforeEach(() => openMock.mockReset());
afterEach(cleanup);

describe("AddWorkspaceModal name suggestion", () => {
  it("suggests the repo folder name when a single repo is added", async () => {
    const onAdd = await renderModal();

    await addRepo("/Users/me/code/payments-api");

    expect(screen.getByPlaceholderText("payments-api")).toBeInTheDocument();

    submit();

    expect(onAdd).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "payments-api",
        repos: [
          expect.objectContaining({
            name: "payments-api",
            localPath: "/Users/me/code/payments-api",
            worktreeBasePath: "/Users/me/Documents/.worktreemanager/worktrees/payments-api",
          }),
        ],
      })
    );
  });

  it("keeps a typed name instead of the suggestion", async () => {
    const onAdd = await renderModal();

    fireEvent.change(nameField(), { target: { value: "Payments" } });
    await addRepo("/Users/me/code/payments-api");

    submit();

    expect(onAdd).toHaveBeenCalledWith(expect.objectContaining({ name: "Payments" }));
  });

  it("stops suggesting once a second repo is added", async () => {
    const onAdd = await renderModal();

    await addRepo("/Users/me/code/payments-api");
    expect(screen.getByPlaceholderText("payments-api")).toBeInTheDocument();

    await addRepo("/Users/me/code/payments-web");
    expect(nameField()).toBeInTheDocument();

    submit();

    expect(onAdd).not.toHaveBeenCalled();
    expect(screen.getByText("Workspace name is required")).toBeInTheDocument();
  });
});
