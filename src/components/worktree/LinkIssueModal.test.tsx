// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { LinkIssueModal } from "./LinkIssueModal";
import { Task } from "../../types";

vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion: vi.fn() }));
const task = { id: "t1", branchName: "feature" } as Task;
afterEach(cleanup);

describe("LinkIssueModal", () => {
  it("links the selected task and closes after success", async () => {
    const onLink = vi.fn().mockResolvedValue(task);
    const onClose = vi.fn();
    render(<LinkIssueModal task={task} configured onLink={onLink} onClose={onClose} />);
    expect(screen.getByRole("button", { name: "Link issue" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Linear issue ID"), { target: { value: " WOR-123 " } });
    fireEvent.click(screen.getByRole("button", { name: "Link issue" }));
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
    expect(onLink).toHaveBeenCalledWith("t1", "WOR-123");
  });

  it("shows errors and allows retry without closing", async () => {
    const onLink = vi
      .fn()
      .mockRejectedValueOnce(new Error("Issue unavailable"))
      .mockResolvedValue(task);
    const onClose = vi.fn();
    render(<LinkIssueModal task={task} configured onLink={onLink} onClose={onClose} />);
    fireEvent.change(screen.getByLabelText("Linear issue ID"), { target: { value: "WOR-123" } });
    fireEvent.click(screen.getByRole("button", { name: "Link issue" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Issue unavailable");
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Link issue" }));
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
  });

  it("explains missing configuration and prevents submission", () => {
    render(<LinkIssueModal task={task} configured={false} onLink={vi.fn()} onClose={vi.fn()} />);
    expect(screen.getByText("Configure Linear in this workspace first.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Link issue" })).toBeDisabled();
  });
});
