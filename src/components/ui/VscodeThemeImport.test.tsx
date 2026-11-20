// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { VscodeThemeImport } from "./VscodeThemeImport";
import { themePackage } from "../../vsix.fixture";

const DOWNLOAD = "https://open-vsx.org/api/acme/night/1.0.0/file/acme.night-1.0.0.vsix";

const importSpy = () => vi.fn<(colors: Record<string, string>) => void>();

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function chooseFile(file: File) {
  fireEvent.change(screen.getByLabelText("VS Code theme file"), { target: { files: [file] } });
}

function search(query: string) {
  fireEvent.change(screen.getByLabelText("Search Open VSX themes"), { target: { value: query } });
  fireEvent.click(screen.getByRole("button", { name: "Search" }));
}

it("applies a single theme file right away", async () => {
  const onImport = importSpy();
  render(<VscodeThemeImport onImport={onImport} />);

  chooseFile(new File(['{"colors": {"editor.background": "#123456"}}'], "deep.json"));

  await waitFor(() => expect(onImport).toHaveBeenCalledTimes(1));
  expect(onImport.mock.calls[0][0]["bg-primary"]).toBe("#123456");
  expect(screen.getByText("Themes in deep.json")).toBeVisible();
});

it("lets the user pick among the themes of a package file", async () => {
  const onImport = importSpy();
  render(<VscodeThemeImport onImport={onImport} />);
  const bytes = await themePackage({
    "a.json": { colors: { "editor.background": "#111111" } },
    "b.json": { colors: { "editor.background": "#222222" } },
  });

  chooseFile(new File([bytes as BlobPart], "pack.VSIX"));
  fireEvent.click(await screen.findByRole("button", { name: "b" }));

  expect(onImport).toHaveBeenCalledTimes(1);
  expect(onImport.mock.calls[0][0]["bg-primary"]).toBe("#222222");
});

it("shows why a file could not be imported", async () => {
  render(<VscodeThemeImport onImport={vi.fn()} />);

  chooseFile(new File(["nope"], "bad.json"));

  expect(await screen.findByRole("alert")).toHaveTextContent("bad.json is not valid JSON.");
});

it("ignores an empty file selection", () => {
  render(<VscodeThemeImport onImport={vi.fn()} />);
  const input = screen.getByLabelText("VS Code theme file");
  const click = vi.spyOn(input, "click");

  fireEvent.click(screen.getByRole("button", { name: "Import file…" }));
  fireEvent.change(input, { target: { files: [] } });

  expect(click).toHaveBeenCalled();
  expect(screen.queryByRole("alert")).toBeNull();
});

it("searches Open VSX only when asked and imports a result", async () => {
  const bytes = await themePackage({
    "night.json": { colors: { "editor.background": "#0b0b0b" } },
  });
  const fetchMock = vi.fn((url: string) =>
    Promise.resolve(
      url === DOWNLOAD
        ? new Response(bytes as BodyInit)
        : Response.json({
            extensions: [
              {
                namespace: "acme",
                name: "night",
                displayName: "Night",
                description: "A dark theme",
                files: { download: DOWNLOAD },
              },
            ],
          })
    )
  );
  vi.stubGlobal("fetch", fetchMock);
  const onImport = importSpy();
  render(<VscodeThemeImport onImport={onImport} />);

  search("   ");
  expect(fetchMock).not.toHaveBeenCalled();

  search("night");
  fireEvent.click(await screen.findByRole("button", { name: /Night.*A dark theme/ }));

  await waitFor(() => expect(onImport).toHaveBeenCalledTimes(1));
  expect(onImport.mock.calls[0][0]["bg-primary"]).toBe("#0b0b0b");
  expect(fetchMock).toHaveBeenCalledTimes(2);
});

it("says when Open VSX has no matching themes", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json({ extensions: [] })));
  render(<VscodeThemeImport onImport={vi.fn()} />);

  search("zzz");

  expect(await screen.findByText("No themes found on Open VSX")).toBeVisible();
});

it("shows a selectable error when Open VSX is unreachable", async () => {
  vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("offline")));
  render(<VscodeThemeImport onImport={vi.fn()} />);

  search("night");

  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("Could not reach Open VSX");
  expect(alert).toHaveClass("select-text");
  expect(screen.getByRole("button", { name: "Search" })).toBeEnabled();
});
