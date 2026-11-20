import { afterEach, describe, expect, it, vi } from "vitest";
import { themePackage } from "../vsix.fixture";
import { loadOpenVsxThemes, OpenVsxExtension, searchOpenVsxThemes } from "./openVsx";

const DOWNLOAD = "https://open-vsx.org/api/acme/night/1.0.0/file/acme.night-1.0.0.vsix";

function stubFetch(respond: (url: string) => Response | Promise<Response>) {
  const fetchMock = vi.fn((url: string) => Promise.resolve(respond(url)));
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

const extension: OpenVsxExtension = {
  id: "acme.night",
  name: "Night",
  publisher: "acme",
  description: "",
  downloadUrl: DOWNLOAD,
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("searchOpenVsxThemes", () => {
  it("searches the themes category and keeps downloadable results", async () => {
    const fetchMock = stubFetch(() =>
      Response.json({
        extensions: [
          {
            namespace: "acme",
            name: "night",
            displayName: " Night Owl ",
            description: "Dark",
            files: { download: DOWNLOAD },
          },
          { namespace: "acme", name: "plain", files: { download: DOWNLOAD } },
          { namespace: "evil", name: "x", files: { download: "https://example.com/x.vsix" } },
          { namespace: "acme", files: {} },
          { namespace: "acme", name: "nofiles" },
          null,
        ],
      })
    );

    const results = await searchOpenVsxThemes("  night owl ");

    const url = new URL(fetchMock.mock.calls[0][0]);
    expect(url.origin + url.pathname).toBe("https://open-vsx.org/api/-/search");
    expect(url.searchParams.get("query")).toBe("night owl");
    expect(url.searchParams.get("category")).toBe("Themes");
    expect(results).toEqual([
      {
        id: "acme.night",
        name: "Night Owl",
        publisher: "acme",
        description: "Dark",
        downloadUrl: DOWNLOAD,
      },
      {
        id: "acme.plain",
        name: "plain",
        publisher: "acme",
        description: "",
        downloadUrl: DOWNLOAD,
      },
    ]);
  });

  it("reports offline, failed and unreadable responses", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Load failed")));
    await expect(searchOpenVsxThemes("x")).rejects.toThrow(
      "Could not reach Open VSX. Check your connection and try again."
    );

    stubFetch(() => new Response("busy", { status: 503 }));
    await expect(searchOpenVsxThemes("x")).rejects.toThrow("Open VSX responded with 503");

    stubFetch(() => new Response("<html>"));
    await expect(searchOpenVsxThemes("x")).rejects.toThrow(
      "Open VSX returned an unreadable response."
    );
  });
});

describe("loadOpenVsxThemes", () => {
  it("downloads the package and reads its themes", async () => {
    const bytes = await themePackage({
      "night.json": { colors: { "editor.background": "#010203" } },
    });
    const fetchMock = stubFetch(() => new Response(bytes as BodyInit));

    const themes = await loadOpenVsxThemes(extension);

    expect(fetchMock.mock.calls[0][0]).toBe(DOWNLOAD);
    expect(themes[0].colors["bg-primary"]).toBe("#010203");
  });

  it("refuses packages that are too large", async () => {
    stubFetch(() => new Response("", { headers: { "content-length": String(30 * 1024 * 1024) } }));
    await expect(loadOpenVsxThemes(extension)).rejects.toThrow(
      "Night is too large to import (over 25 MB)."
    );

    stubFetch(() => new Response(new Uint8Array(26 * 1024 * 1024)));
    await expect(loadOpenVsxThemes(extension)).rejects.toThrow("too large to import");
  });

  it("reports a download that breaks off", async () => {
    const broken = new ReadableStream({
      start(controller) {
        controller.error(new Error("connection reset"));
      },
    });
    stubFetch(() => new Response(broken));
    await expect(loadOpenVsxThemes(extension)).rejects.toThrow("Could not reach Open VSX");
  });
});
