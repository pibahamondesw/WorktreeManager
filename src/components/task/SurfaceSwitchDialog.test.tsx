// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

const useEditorOcclusion = vi.hoisted(() => vi.fn());
vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion }));

import { SurfaceSwitchDialog } from "./SurfaceSwitchDialog";
import { SurfaceSwitchPrompt } from "../../hooks/useOpenTask";
import { AgentActivity } from "../../services/agentActivity";

const prompt: SurfaceSwitchPrompt = {
  phase: "asking",
  taskId: "t1",
  current: { kind: "chat", agent: "claude" },
  requested: { kind: "opencode" },
  closing: [{ kind: "chat", agent: "claude" }],
};

const claudeWorking: AgentActivity = {
  state: "working",
  agent: "claude",
  surface: "chat",
  unread: false,
  at: 1,
};

afterEach(cleanup);

describe("SurfaceSwitchDialog", () => {
  it("renders nothing without a pending switch", () => {
    const { container } = render(<SurfaceSwitchDialog prompt={null} onChoose={vi.fn()} />);
    expect(container.innerHTML).toBe("");
    expect(useEditorOcclusion).not.toHaveBeenCalledWith(true);
  });

  it("explains the switch, hides embedded editors and focuses keeping the live surface", () => {
    const onChoose = vi.fn();
    render(<SurfaceSwitchDialog prompt={prompt} onChoose={onChoose} />);
    expect(
      screen.getByText(
        "This task already has Claude Code chat open. Opening it in OpenCode will close it."
      )
    ).toBeTruthy();
    expect(useEditorOcclusion).toHaveBeenCalledWith(true);
    expect(screen.queryByRole("alert")).toBeNull();
    const keep = screen.getByRole("button", { name: "Keep using Claude Code chat" });
    expect(document.activeElement).toBe(keep);
    fireEvent.click(keep);
    expect(onChoose).toHaveBeenCalledWith("keep");
  });

  it("closes the live surface and opens the requested one on the second action", () => {
    const onChoose = vi.fn();
    render(<SurfaceSwitchDialog prompt={prompt} onChoose={onChoose} />);
    fireEvent.click(
      screen.getByRole("button", { name: "Close Claude Code chat and open OpenCode" })
    );
    expect(onChoose).toHaveBeenCalledWith("replace");
  });

  it("cancels with Escape", () => {
    const onChoose = vi.fn();
    render(<SurfaceSwitchDialog prompt={prompt} onChoose={onChoose} />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onChoose).toHaveBeenCalledWith("cancel");
  });

  it("warns that closing interrupts an agent working there", () => {
    render(
      <SurfaceSwitchDialog prompt={prompt} agentActivity={claudeWorking} onChoose={vi.fn()} />
    );
    expect(screen.getByRole("alert").textContent).toContain(
      "Claude Code is working in this task; closing it will interrupt its turn."
    );
  });

  it("blocks the choices while closing and shows a failure to close", () => {
    const view = render(
      <SurfaceSwitchDialog prompt={{ ...prompt, phase: "closing" }} onChoose={vi.fn()} />
    );
    const keep = screen.getByRole("button", { name: "Keep using Claude Code chat" });
    expect((keep as HTMLButtonElement).disabled).toBe(true);
    view.rerender(
      <SurfaceSwitchDialog
        prompt={{ ...prompt, error: "Could not close Claude Code chat: busy" }}
        onChoose={vi.fn()}
      />
    );
    expect(screen.getByText("Could not close Claude Code chat: busy")).toBeTruthy();
    expect((keep as HTMLButtonElement).disabled).toBe(false);
  });

  it("gives focus back to where it was when it closes", () => {
    const input = document.createElement("input");
    document.body.append(input);
    input.focus();
    const view = render(<SurfaceSwitchDialog prompt={prompt} onChoose={vi.fn()} />);
    expect(document.activeElement).not.toBe(input);
    view.rerender(<SurfaceSwitchDialog prompt={null} onChoose={vi.fn()} />);
    expect(document.activeElement).toBe(input);
    input.remove();
  });
});
