// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { AddWorkspaceModal } from "./AddWorkspaceModal";

type DropPayload = { type: "enter" | "drop"; paths: string[]; position: { x: number; y: number } };

const drops = vi.hoisted(() => ({
  handlers: new Set<(event: { payload: DropPayload }) => void>(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: () => Promise.resolve("/Users/me/") }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (handler: (event: { payload: DropPayload }) => void) => {
      drops.handlers.add(handler);
      return Promise.resolve(() => drops.handlers.delete(handler));
    },
  }),
}));

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
const submit = () => fireEvent.click(screen.getByText("Add workspace", { selector: "button" }));

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

describe("AddWorkspaceModal duplicate repos", () => {
  it("warns and does not add a repo that is already in the workspace", async () => {
    const onAdd = await renderModal();

    await addRepo("/Users/me/code/payments-api");
    await addRepo("/Users/me/code/payments-api/");

    expect(screen.getByRole("alert")).toHaveTextContent(
      '"payments-api" is already in this workspace.'
    );
    expect(screen.getAllByDisplayValue("payments-api")).toHaveLength(1);

    submit();

    expect(onAdd).toHaveBeenCalledWith(
      expect.objectContaining({ repos: [expect.objectContaining({ name: "payments-api" })] })
    );
  });

  it("clears the warning when a different repo is added", async () => {
    await renderModal();

    await addRepo("/Users/me/code/payments-api");
    await addRepo("/Users/me/code/payments-api");
    expect(screen.getByRole("alert")).toBeInTheDocument();

    await addRepo("/Users/me/code/payments-web");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});

describe("AddWorkspaceModal dropped folders", () => {
  async function dropFolders(paths: string[]) {
    const target = screen.getByText("Repositories");
    document.elementFromPoint = () => target;
    const position = { x: 20, y: 20 };
    await act(async () => {
      drops.handlers.forEach((handler) => handler({ payload: { type: "enter", paths, position } }));
      drops.handlers.forEach((handler) => handler({ payload: { type: "drop", paths, position } }));
    });
  }

  it("adds each dropped folder as a repo and skips ones already added", async () => {
    const onAdd = await renderModal();

    await addRepo("/Users/me/code/payments-api");
    await dropFolders(["/Users/me/code/payments-api", "/Users/me/code/payments-web"]);

    expect(screen.getByRole("alert")).toHaveTextContent(
      '"payments-api" is already in this workspace.'
    );
    fireEvent.change(nameField(), { target: { value: "Payments" } });
    submit();

    expect(onAdd).toHaveBeenCalledWith(
      expect.objectContaining({
        repos: [
          expect.objectContaining({ localPath: "/Users/me/code/payments-api" }),
          expect.objectContaining({
            name: "payments-web",
            localPath: "/Users/me/code/payments-web",
          }),
        ],
      })
    );
  });

  it("accepts Windows paths", async () => {
    const onAdd = await renderModal();

    await dropFolders(["C:\\Users\\me\\code\\payments-api"]);
    submit();

    expect(onAdd).toHaveBeenCalledWith(
      expect.objectContaining({
        repos: [
          expect.objectContaining({
            name: "payments-api",
            localPath: "C:\\Users\\me\\code\\payments-api",
          }),
        ],
      })
    );
  });
});
