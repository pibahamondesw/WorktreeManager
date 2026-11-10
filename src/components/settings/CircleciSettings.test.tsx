// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { CircleciSettings } from "./CircleciSettings";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

afterEach(() => {
  cleanup();
  vi.resetAllMocks();
});

function answer(saved: boolean | Error, save: Error | null = null) {
  invoke.mockImplementation((command: string) => {
    if (command === "circleci_token_saved")
      return saved instanceof Error ? Promise.reject(saved) : Promise.resolve(saved);
    return save ? Promise.reject(save) : Promise.resolve();
  });
}

const tokenField = () => screen.getByLabelText("Personal API token");

describe("CircleciSettings", () => {
  it("saves a token without ever reading it back", async () => {
    answer(false);
    render(<CircleciSettings />);
    await waitFor(() => expect(tokenField()).toHaveAttribute("placeholder", "No token saved"));
    expect(screen.getByRole("button", { name: "Save token" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Remove token" })).not.toBeInTheDocument();

    fireEvent.change(tokenField(), { target: { value: "circle-token" } });
    fireEvent.click(screen.getByRole("button", { name: "Save token" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("circleci_token_save", { value: "circle-token" })
    );
    await waitFor(() => expect(tokenField()).toHaveValue(""));
    expect(tokenField()).toHaveAttribute("type", "password");
    expect(screen.getByRole("button", { name: "Remove token" })).toBeInTheDocument();
  });

  it("removes a saved token", async () => {
    answer(true);
    render(<CircleciSettings />);
    fireEvent.click(await screen.findByRole("button", { name: "Remove token" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("circleci_token_save", { value: null })
    );
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Remove token" })).not.toBeInTheDocument()
    );
  });

  it("shows Keychain failures", async () => {
    answer(new Error("Failed to read from keychain: denied"));
    render(<CircleciSettings />);
    expect(await screen.findByText("Error: Failed to read from keychain: denied")).toBeVisible();

    cleanup();
    answer(true, new Error("Failed to write to keychain: denied"));
    render(<CircleciSettings />);
    fireEvent.change(tokenField(), { target: { value: "circle-token" } });
    fireEvent.click(screen.getByRole("button", { name: "Save token" }));
    expect(await screen.findByText("Error: Failed to write to keychain: denied")).toBeVisible();
    expect(tokenField()).toHaveValue("circle-token");
  });
});
