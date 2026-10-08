// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { Modal } from "./Modal";
vi.mock("../../hooks/useEditorOcclusion", () => ({ useEditorOcclusion: vi.fn() }));
afterEach(cleanup);
it("Escape closes only the topmost dialog", () => {
  const lower = vi.fn();
  const upper = vi.fn();
  render(
    <>
      <Modal open title="Lower" onClose={lower}>
        First
      </Modal>
      <Modal open title="Upper" onClose={upper}>
        Second
      </Modal>
    </>
  );
  fireEvent.keyDown(document, { key: "Escape" });
  expect(upper).toHaveBeenCalledOnce();
  expect(lower).not.toHaveBeenCalled();
});
