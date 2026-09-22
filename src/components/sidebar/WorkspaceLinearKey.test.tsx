// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { validateLinearToken } from "../../services/linear";
import { Workspace } from "../../types";
import { EditWorkspaceModal } from "./EditWorkspaceModal";
import { AddWorkspaceModal } from "./AddWorkspaceModal";

vi.mock("../../services/linear", () => ({ validateLinearToken: vi.fn() }));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: () => Promise.resolve("/Users/me/") }));
vi.mock("./WorkspaceRepoEditor", () => ({
  WorkspaceRepoEditor: () => null,
}));

const workspace: Workspace = {
  id: "target",
  name: "Target",
  repos: [{ id: "repo", name: "Repo", localPath: "/repo", worktreeBasePath: "/worktrees" }],
};
const source: Workspace = {
  ...workspace,
  id: "source",
  name: "Source",
  linearApiKey: "saved-key",
  linearOrgUrlKey: "my-org",
};
const validate = vi.mocked(validateLinearToken);

beforeEach(() => {
  validate.mockReset();
  validate.mockResolvedValue({ valid: true, name: "Person", orgUrlKey: "my-org" });
});
afterEach(cleanup);

async function edit(target = workspace) {
  const onSave = vi.fn();
  render(
    <EditWorkspaceModal
      open
      workspace={target}
      workspaces={[target, source]}
      defaultLinearApiKey="saved-key"
      onSave={onSave}
      onClose={vi.fn()}
      onRequestDelete={vi.fn()}
    />
  );
  await act(async () => {});
  return onSave;
}

async function select(value: string) {
  await act(async () => {
    fireEvent.change(screen.getByRole("combobox"), { target: { value } });
  });
}

describe("workspace Linear connections", () => {
  it("deduplicates the default and labels it with its organization and source", async () => {
    await edit();
    expect(screen.getAllByRole("option")).toHaveLength(3);
    expect(screen.getByRole("option", { name: "my-org — Default, Source" })).toHaveValue("0");
    expect(screen.getByRole("combobox")).toHaveValue("none");
    expect(screen.getByRole("combobox").innerHTML).not.toContain("saved-key");
  });

  it.each([null, "deleted-key"])("connects a workspace whose previous key is %s", async (key) => {
    const onSave = await edit({ ...workspace, linearApiKey: key });
    await select("0");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).toHaveBeenCalledWith(
      "target",
      expect.objectContaining({
        linearApiKey: "saved-key",
        linearOrgUrlKey: "my-org",
      })
    );
  });

  it("does not save an invalid replacement", async () => {
    const onSave = await edit({ ...workspace, linearApiKey: "original" });
    validate.mockResolvedValue({ valid: false, error: "Revoked" });
    await select("0");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).not.toHaveBeenCalled();
    expect(screen.getByText("Validate the Linear API key before saving")).toBeInTheDocument();
  });

  it("blocks saving a replacement while validation is pending", async () => {
    const onSave = await edit();
    validate.mockReturnValue(new Promise(() => {}));
    await select("0");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).not.toHaveBeenCalled();
    await select("none");
    expect(screen.getByRole("button", { name: "Validate" })).toBeDisabled();
  });

  it("preserves an unchanged connection if validation fails", async () => {
    validate.mockResolvedValue({ valid: false, error: "Offline" });
    const onSave = await edit({
      ...workspace,
      linearApiKey: "original",
      linearOrgUrlKey: "original-org",
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).toHaveBeenCalledWith(
      "target",
      expect.objectContaining({
        linearApiKey: "original",
        linearOrgUrlKey: "original-org",
      })
    );
  });

  it("explicitly disconnects without a stale validation reconnecting it", async () => {
    let resolve!: (value: { valid: boolean; orgUrlKey: string }) => void;
    validate.mockReturnValue(
      new Promise((done) => {
        resolve = done;
      })
    );
    const onSave = await edit({ ...workspace, linearApiKey: "original" });
    await select("none");
    await act(async () => {
      resolve({ valid: true, orgUrlKey: "old-org" });
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).toHaveBeenCalledWith(
      "target",
      expect.objectContaining({
        linearApiKey: null,
        linearOrgUrlKey: null,
      })
    );
  });

  it("can enter a new key after choosing the custom option", async () => {
    await edit();
    await select("custom");
    expect(screen.getByRole("combobox")).toHaveValue("custom");
    fireEvent.change(screen.getByPlaceholderText("lin_api_..."), { target: { value: "new-key" } });
    fireEvent.click(screen.getByRole("button", { name: "Validate" }));
    await waitFor(() => expect(screen.getByText("Connected as Person")).toBeInTheDocument());
    expect(validate).toHaveBeenCalledWith("new-key");
  });

  it("preselects the default when creating and allows another workspace key", async () => {
    render(
      <AddWorkspaceModal
        open
        onClose={vi.fn()}
        onAdd={vi.fn()}
        workspaces={[source]}
        defaultLinearApiKey="default-key"
      />
    );
    await act(async () => {});
    expect(screen.getByRole("combobox")).toHaveValue("0");
    await select("1");
    expect(validate).toHaveBeenLastCalledWith("saved-key");
    expect(screen.getByPlaceholderText("lin_api_...")).toHaveValue("saved-key");
  });
});
