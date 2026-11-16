// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { openUrl } from "@tauri-apps/plugin-opener";
import { PendingRequestCard } from "./PendingRequestCard";
import { PendingRequest } from "../../services/chat";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
beforeEach(() => vi.mocked(openUrl).mockReset().mockResolvedValue(undefined));
afterEach(cleanup);

const form: PendingRequest = {
  id: "elicitation:1",
  kind: "elicitation",
  title: "MCP · browser",
  serverName: "browser",
  format: "text",
  detail: "Choose download settings",
  mode: "form",
  requestedSchema: {
    type: "object",
    required: ["filename", "overwrite", "copies"],
    properties: {
      filename: { type: "string", title: "Filename", minLength: 3 },
      overwrite: { type: "boolean", title: "Overwrite" },
      copies: { type: "integer", title: "Copies", minimum: 1, maximum: 5, default: 1 },
      folder: {
        type: "string",
        oneOf: [
          { const: "docs", title: "Documents" },
          { const: "tmp", title: "Temporary" },
        ],
      },
      tags: { type: "array", items: { enum: ["a", "b"] } },
      note: { type: "string" },
    },
  },
};
const url: PendingRequest = {
  id: "elicitation:2",
  kind: "elicitation",
  title: "MCP · browser",
  serverName: "browser",
  format: "text",
  mode: "url",
  detail: "Sign in to download",
  url: "https://example.com/auth?request=one",
  elicitationId: "login-1",
};

it("shows the server and message and submits schema values with their original types", async () => {
  const onRespond = vi.fn().mockResolvedValue(undefined);
  render(<PendingRequestCard request={form} onRespond={onRespond} />);
  expect(screen.getByRole("region", { name: "MCP · browser" })).toBeInTheDocument();
  expect(screen.getByText("Choose download settings")).toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: "Filename" }), {
    target: { value: "report.pdf" },
  });
  fireEvent.change(screen.getByRole("combobox", { name: "Overwrite" }), {
    target: { value: "false" },
  });
  fireEvent.change(screen.getByRole("combobox", { name: "folder" }), {
    target: { value: '"docs"' },
  });
  const tags = screen.getByRole("listbox", { name: "tags" });
  for (const option of Array.from(tags.querySelectorAll("option"))) option.selected = true;
  fireEvent.change(tags);
  fireEvent.click(screen.getByRole("button", { name: "Accept" }));
  await waitFor(() =>
    expect(onRespond).toHaveBeenCalledExactlyOnceWith({
      elicitation: {
        action: "accept",
        content: {
          filename: "report.pdf",
          overwrite: false,
          copies: 1,
          folder: "docs",
          tags: ["a", "b"],
        },
      },
    })
  );
});

it("blocks missing required values and numbers outside the schema bounds", () => {
  const onRespond = vi.fn();
  render(<PendingRequestCard request={form} onRespond={onRespond} />);
  fireEvent.click(screen.getByRole("button", { name: "Accept" }));
  expect(onRespond).not.toHaveBeenCalled();
  fireEvent.change(screen.getByRole("textbox", { name: "Filename" }), {
    target: { value: "report.pdf" },
  });
  fireEvent.change(screen.getByRole("combobox", { name: "Overwrite" }), {
    target: { value: "true" },
  });
  fireEvent.change(screen.getByRole("spinbutton", { name: "Copies" }), { target: { value: "6" } });
  fireEvent.click(screen.getByRole("button", { name: "Accept" }));
  expect(onRespond).not.toHaveBeenCalled();
});

it("accepts required empty strings and arrays when the schema permits them", async () => {
  const onRespond = vi.fn().mockResolvedValue(undefined);
  render(
    <PendingRequestCard
      request={{
        ...form,
        requestedSchema: {
          type: "object",
          required: ["empty", "tags"],
          properties: {
            empty: { type: "string", maxLength: 0 },
            tags: { type: "array", minItems: 0, items: { enum: ["a", "b"] } },
            defaultEmpty: { type: "string", default: "" },
          },
        },
      }}
      onRespond={onRespond}
    />
  );
  fireEvent.click(screen.getByRole("button", { name: "Accept" }));
  await waitFor(() =>
    expect(onRespond).toHaveBeenCalledWith({
      elicitation: {
        action: "accept",
        content: { empty: "", tags: [], defaultEmpty: "" },
      },
    })
  );
});

it("requires a selection when a required array has a positive minimum", () => {
  const onRespond = vi.fn();
  render(
    <PendingRequestCard
      request={{
        ...form,
        requestedSchema: {
          type: "object",
          required: ["tags"],
          properties: {
            tags: { type: "array", minItems: 1, items: { enum: ["a", "b"] } },
          },
        },
      }}
      onRespond={onRespond}
    />
  );
  fireEvent.click(screen.getByRole("button", { name: "Accept" }));
  expect(onRespond).not.toHaveBeenCalled();
});

it.each([
  ["Decline", "decline"],
  ["Cancel", "cancel"],
])("%s bypasses form validation and sends null content", async (button, action) => {
  const onRespond = vi.fn().mockResolvedValue(undefined);
  render(<PendingRequestCard request={form} onRespond={onRespond} />);
  fireEvent.click(screen.getByRole("button", { name: button }));
  await waitFor(() =>
    expect(onRespond).toHaveBeenCalledExactlyOnceWith({ elicitation: { action, content: null } })
  );
});

it("keeps entered values when backend validation fails and allows a corrected retry", async () => {
  const onRespond = vi
    .fn()
    .mockRejectedValueOnce("Form value does not satisfy /properties/email/format")
    .mockResolvedValue(undefined);
  render(
    <PendingRequestCard
      request={{
        ...form,
        requestedSchema: { type: "object", properties: { email: { type: "string" } } },
      }}
      onRespond={onRespond}
    />
  );
  fireEvent.change(screen.getByRole("textbox", { name: "email" }), {
    target: { value: "invalid" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Accept" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("/properties/email/format");
  expect(screen.getByRole("textbox", { name: "email" })).toHaveValue("invalid");
  fireEvent.change(screen.getByRole("textbox", { name: "email" }), {
    target: { value: "a@b.com" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Accept" }));
  await waitFor(() =>
    expect(onRespond).toHaveBeenLastCalledWith({
      elicitation: { action: "accept", content: { email: "a@b.com" } },
    })
  );
});

it("accepts structured extended forms and reports invalid JSON without responding", async () => {
  const onRespond = vi.fn().mockResolvedValue(undefined);
  render(
    <PendingRequestCard
      request={{ ...form, requestedSchema: { anyOf: [{ type: "object" }, { type: "array" }] } }}
      onRespond={onRespond}
    />
  );
  const input = screen.getByRole("textbox", { name: "Form data (JSON)" });
  fireEvent.change(input, { target: { value: "{" } });
  fireEvent.click(screen.getByRole("button", { name: "Accept" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("valid JSON");
  expect(onRespond).not.toHaveBeenCalled();
  fireEvent.change(input, { target: { value: '{"nested":{"allowed":true}}' } });
  fireEvent.click(screen.getByRole("button", { name: "Accept" }));
  await waitFor(() =>
    expect(onRespond).toHaveBeenCalledExactlyOnceWith({
      elicitation: { action: "accept", content: { nested: { allowed: true } } },
    })
  );
});

it("allows editing the entire object to satisfy required additional properties", async () => {
  const onRespond = vi.fn().mockResolvedValue(undefined);
  render(
    <PendingRequestCard
      request={{
        ...form,
        requestedSchema: {
          type: "object",
          properties: {},
          required: ["extra"],
          additionalProperties: { type: "string" },
        },
      }}
      onRespond={onRespond}
    />
  );
  fireEvent.click(screen.getByRole("button", { name: "Edit as JSON" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Form data (JSON)" }), {
    target: { value: '{"extra":"download"}' },
  });
  fireEvent.click(screen.getByRole("button", { name: "Accept" }));
  await waitFor(() =>
    expect(onRespond).toHaveBeenCalledExactlyOnceWith({
      elicitation: { action: "accept", content: { extra: "download" } },
    })
  );
});

it("preserves typed field values when switching to the full JSON editor", () => {
  render(<PendingRequestCard request={form} onRespond={vi.fn()} />);
  fireEvent.change(screen.getByRole("textbox", { name: "Filename" }), {
    target: { value: "report.pdf" },
  });
  fireEvent.change(screen.getByRole("combobox", { name: "Overwrite" }), {
    target: { value: "false" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Edit as JSON" }));
  const input = screen.getByRole("textbox", { name: "Form data (JSON)" });
  expect(input).toHaveValue(
    JSON.stringify({ filename: "report.pdf", overwrite: false, copies: 1 }, null, 2)
  );
});

it("preserves invalid field input and reports it before switching to JSON", async () => {
  render(
    <PendingRequestCard
      request={{
        ...form,
        requestedSchema: {
          type: "object",
          properties: { settings: { type: "object" } },
        },
      }}
      onRespond={vi.fn()}
    />
  );
  fireEvent.change(screen.getByRole("textbox", { name: "settings" }), { target: { value: "{" } });
  fireEvent.click(screen.getByRole("button", { name: "Edit as JSON" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("valid JSON");
  expect(screen.getByRole("textbox", { name: "settings" })).toHaveValue("{");
});

it("opens a URL only when asked and requires a separate confirmation", async () => {
  const onRespond = vi.fn().mockResolvedValue(undefined);
  render(<PendingRequestCard request={url} onRespond={onRespond} />);
  expect(screen.getByText(url.url)).toBeInTheDocument();
  expect(openUrl).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Open URL" }));
  await waitFor(() => expect(openUrl).toHaveBeenCalledExactlyOnceWith(url.url));
  expect(onRespond).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Confirm completed" }));
  await waitFor(() =>
    expect(onRespond).toHaveBeenCalledExactlyOnceWith({
      elicitation: { action: "accept", content: null },
    })
  );
});

it("shows browser failures and lets the user cancel explicitly", async () => {
  vi.mocked(openUrl).mockRejectedValueOnce(new Error("Browser unavailable"));
  const onRespond = vi.fn().mockResolvedValue(undefined);
  render(<PendingRequestCard request={url} onRespond={onRespond} />);
  fireEvent.click(screen.getByRole("button", { name: "Open URL" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not open the URL");
  expect(onRespond).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await waitFor(() =>
    expect(onRespond).toHaveBeenCalledWith({ elicitation: { action: "cancel", content: null } })
  );
});

it("sends once while a response is in flight and obeys disabled state", async () => {
  let finish!: () => void;
  const onRespond = vi.fn(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      })
  );
  const view = render(<PendingRequestCard request={url} onRespond={onRespond} />);
  fireEvent.click(screen.getByRole("button", { name: "Confirm completed" }));
  fireEvent.click(screen.getByRole("button", { name: "Confirm completed" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onRespond).toHaveBeenCalledTimes(1);
  finish();
  await waitFor(() => expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled());
  view.rerender(<PendingRequestCard request={url} disabled onRespond={onRespond} />);
  fireEvent.click(screen.getByRole("button", { name: "Decline" }));
  expect(onRespond).toHaveBeenCalledTimes(1);
});
