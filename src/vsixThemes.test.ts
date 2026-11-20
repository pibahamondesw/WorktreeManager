import { describe, expect, it } from "vitest";
import { themePackage, vsixPackage } from "./vsix.fixture";
import { themesFromVsix } from "./vsixThemes";

const manifest = (themes: unknown[]) => JSON.stringify({ contributes: { themes } });

describe("themesFromVsix", () => {
  it("reads every JSON theme the extension contributes", async () => {
    const bytes = await themePackage({
      "themes/night.json": { colors: { "editor.background": "#101010" } },
      "themes/day.json": { type: "light", colors: { "editor.background": "#fafafa" } },
    });
    const themes = await themesFromVsix(bytes);
    expect(themes.map((t) => [t.name, t.colors["bg-primary"]])).toEqual([
      ["night", "#101010"],
      ["day", "#fafafa"],
    ]);
  });

  it("merges included themes, letting the including theme win", async () => {
    const bytes = await vsixPackage(
      {
        "extension/package.json": manifest([
          { uiTheme: "vs", path: "./themes/child.json" },
          { label: "Icons", path: "./icons.tmTheme" },
          null,
        ]),
        "extension/themes/child.json": JSON.stringify({
          name: "Child",
          include: "../base/./parent.json",
          colors: { "editor.background": "#eeeeee" },
        }),
        "extension/base/parent.json": JSON.stringify({
          colors: { "editor.background": "#000000", "button.background": "#ff0000" },
        }),
      },
      { stored: true }
    );
    const [theme, ...rest] = await themesFromVsix(bytes);
    expect(rest).toEqual([]);
    expect(theme.name).toBe("Child");
    expect(theme.colors["bg-primary"]).toBe("#eeeeee");
    expect(theme.colors.accent).toBe("#ff0000");
  });

  it("stops following includes that loop", async () => {
    const bytes = await vsixPackage({
      "extension/package.json": manifest([{ label: "Loop", path: "loop.json" }]),
      "extension/loop.json": JSON.stringify({ include: "./loop.json", colors: "none" }),
    });
    const [theme] = await themesFromVsix(bytes);
    expect(theme.name).toBe("Loop");
  });

  it("explains packages without themes or with missing files", async () => {
    await expect(
      themesFromVsix(await vsixPackage({ "extension/package.json": "{}" }))
    ).rejects.toThrow("This extension has no JSON color themes.");
    await expect(
      themesFromVsix(
        await vsixPackage({ "extension/package.json": manifest([{ path: "a.json" }]) })
      )
    ).rejects.toThrow("The extension package is missing extension/a.json.");
    await expect(themesFromVsix(await vsixPackage({ "readme.md": "hi" }))).rejects.toThrow(
      "missing extension/package.json"
    );
  });

  it("rejects files that are not readable zip archives", async () => {
    const unreadable = "This extension package is not a readable VSIX file.";
    await expect(themesFromVsix(new TextEncoder().encode("not a zip at all"))).rejects.toThrow(
      unreadable
    );

    const valid = await vsixPackage({ "extension/package.json": "{}" }, { stored: true });
    const brokenCentral = valid.slice();
    const centralOffset = new DataView(brokenCentral.buffer).getUint32(valid.length - 6, true);
    brokenCentral[centralOffset] = 0;
    await expect(themesFromVsix(brokenCentral)).rejects.toThrow(unreadable);

    const brokenLocal = valid.slice();
    brokenLocal[0] = 0;
    await expect(themesFromVsix(brokenLocal)).rejects.toThrow(unreadable);

    const unknownMethod = valid.slice();
    new DataView(unknownMethod.buffer).setUint16(centralOffset + 10, 12, true);
    await expect(themesFromVsix(unknownMethod)).rejects.toThrow(unreadable);

    const deflated = await vsixPackage({ "extension/package.json": "{}" });
    const corrupt = deflated.slice();
    corrupt.fill(
      0xff,
      30 + "extension/package.json".length,
      30 + "extension/package.json".length + 2
    );
    await expect(themesFromVsix(corrupt)).rejects.toThrow(unreadable);
  });
});
