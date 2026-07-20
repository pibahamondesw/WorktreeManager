// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { SetupWizard } from "./SetupWizard";
import { AppState } from "../../types";

const emptySetup: AppState["setup"] = { linearApiKey: null, isComplete: false };

afterEach(cleanup);

describe("SetupWizard", () => {
  it("shows the app logo instead of a generic icon", () => {
    render(<SetupWizard initialSetup={emptySetup} onComplete={vi.fn()} />);

    const logo = screen.getByAltText("WorktreeManager");
    expect(logo.tagName).toBe("IMG");
    expect(logo).toHaveAttribute("src", expect.stringContaining("logo"));
  });

  it("completes with no key when the user skips", () => {
    const onComplete = vi.fn();
    render(<SetupWizard initialSetup={emptySetup} onComplete={onComplete} />);

    fireEvent.click(screen.getByText("Skip for now"));

    expect(onComplete).toHaveBeenCalledWith({ linearApiKey: null, isComplete: true });
  });
});
