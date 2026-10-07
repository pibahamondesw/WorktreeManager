// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { LinearKeyField } from "./LinearKeyField";

afterEach(cleanup);

it("associates the API-key label with its password field", () => {
  render(
    <LinearKeyField
      linearKey=""
      linearValid={false}
      linearUser={null}
      linearValidating={false}
      linearError={null}
      onChange={vi.fn()}
      onValidate={vi.fn()}
    />
  );
  expect(screen.getByLabelText("Linear API key (optional)")).toHaveAttribute("type", "password");
});
