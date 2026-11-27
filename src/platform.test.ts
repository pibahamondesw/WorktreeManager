import { expect, it } from "vitest";
import { customProtocolUrl } from "./platform";

it("addresses custom protocols as a scheme on macOS and Linux", () => {
  expect(customProtocolUrl("wtm-preview", false)).toBe("wtm-preview://localhost/");
});

it("addresses custom protocols as a localhost subdomain on Windows", () => {
  expect(customProtocolUrl("wtm-preview", true)).toBe("http://wtm-preview.localhost/");
});
