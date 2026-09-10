// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import "@testing-library/jest-dom/vitest";

const mocks = vi.hoisted(() => ({ install: vi.fn(), status: vi.fn() }));
vi.mock("../../services/codeEditor", () => ({
  editorInstall: mocks.install,
  editorInstallStatus: mocks.status,
}));
import { EditorInstallation } from "./EditorInstallation";

beforeEach(() => {
  vi.resetAllMocks();
});
afterEach(cleanup);

it("shows an installation already running in another view with byte progress and logs", async () => {
  mocks.status.mockResolvedValue({
    active: true,
    phase: "downloading",
    message: "Downloading the official editor…",
    downloaded: 1048576,
    total: 2097152,
    startedAt: Date.now() - 5000,
    logs: ["Checking", "Downloading"],
  });
  render(<EditorInstallation onReady={vi.fn()} />);
  expect(await screen.findByText("1.0 MB / 2.0 MB")).toBeInTheDocument();
  expect(screen.getByRole("button")).toBeDisabled();
  expect(screen.getByRole("progressbar")).toHaveAttribute("value", "1048576");
  expect(screen.getByRole("status")).toHaveTextContent("5s");
  expect(mocks.install).not.toHaveBeenCalled();
});

it("surfaces failures and allows retry without depending on extensions", async () => {
  mocks.status.mockResolvedValue({ active: false, phase: "", message: "", logs: [] });
  mocks.install
    .mockRejectedValueOnce(new Error("Download failed"))
    .mockResolvedValueOnce({ ready: true });
  const ready = vi.fn();
  render(<EditorInstallation onReady={ready} />);
  fireEvent.click(screen.getByRole("button", { name: "Install editor" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Download failed");
  fireEvent.click(screen.getByRole("button", { name: "Install editor" }));
  await waitFor(() => expect(ready).toHaveBeenCalledOnce());
});

it("notifies the current view when another view completed installation", async () => {
  mocks.status.mockResolvedValue({
    active: false,
    phase: "ready",
    message: "Editor ready",
    logs: [],
  });
  const ready = vi.fn();
  render(<EditorInstallation onReady={ready} />);
  await waitFor(() => expect(ready).toHaveBeenCalledOnce());
  expect(mocks.install).not.toHaveBeenCalled();
});
