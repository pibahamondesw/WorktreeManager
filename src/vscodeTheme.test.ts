import { describe, expect, it } from "vitest";
import { colorsFromVscodeTheme, parseThemeDocument, themeFromFile } from "./vscodeTheme";
import { generateThemeColors, mixColors } from "./themePalette";

describe("parseThemeDocument", () => {
  it("accepts comments and trailing commas but keeps them inside strings", () => {
    const source = `{
      // a comment
      "name": "Slashes // and /* not comments */ ,}",
      /* block
         comment */
      "colors": { "editor.background": "#112233", },
      "escaped": "quote \\" inside",
    }`;
    expect(parseThemeDocument(source, "theme.json")).toEqual({
      name: "Slashes // and /* not comments */ ,}",
      colors: { "editor.background": "#112233" },
      escaped: 'quote " inside',
    });
  });

  it("tolerates unterminated comments at the end of the file", () => {
    expect(parseThemeDocument('{"a": [1, 2,]} // end', "t.json")).toEqual({ a: [1, 2] });
    expect(parseThemeDocument('{"a": 1} /* open', "t.json")).toEqual({ a: 1 });
  });

  it("names the file when it is not JSON or not an object", () => {
    expect(() => parseThemeDocument("{nope", "broken.json")).toThrow(
      "broken.json is not valid JSON."
    );
    expect(() => parseThemeDocument("[1]", "list.json")).toThrow(
      "list.json is not a VS Code color theme."
    );
    expect(() => parseThemeDocument("null", "null.json")).toThrow("not a VS Code color theme");
  });
});

describe("colorsFromVscodeTheme", () => {
  it("maps workbench colors onto app tokens, compositing alpha over the editor", () => {
    const colors = colorsFromVscodeTheme({
      type: "dark",
      colors: {
        "editor.background": "#1e1e1e",
        "sideBar.background": "#252526",
        "list.hoverBackground": "#ffffff1a",
        "editor.foreground": "#d4d4d4",
        "button.background": "#0e639c",
        "button.hoverBackground": "#1177bb",
        "terminal.ansiGreen": "#0dbc79",
        "editorWarning.foreground": "#cca700",
        errorForeground: "#f48771",
      },
    });
    expect(colors["bg-primary"]).toBe("#1e1e1e");
    expect(colors["bg-secondary"]).toBe("#252526");
    expect(colors["bg-hover"]).toBe(mixColors("#1e1e1e", "#ffffff", 0x1a / 255));
    expect(colors["text-primary"]).toBe("#d4d4d4");
    expect(colors.accent).toBe("#0e639c");
    expect(colors["accent-hover"]).toBe("#1177bb");
    expect(colors.success).toBe("#0dbc79");
    expect(colors.warning).toBe("#cca700");
    expect(colors.danger).toBe("#f48771");
    expect(colors["danger-hover"]).toBe(mixColors("#f48771", "#000000", 0.15));
  });

  it("generates the whole palette when a theme only sets token colors", () => {
    expect(colorsFromVscodeTheme({ type: "light", colors: null })).toEqual(
      generateThemeColors("#ffffff", "#007acc")
    );
    expect(colorsFromVscodeTheme({})).toEqual(generateThemeColors("#1e1e1e", "#007acc"));
    expect(colorsFromVscodeTheme({ type: "vs", colors: { focusBorder: "#ff0000" } }).accent).toBe(
      "#ff0000"
    );
  });
});

describe("themeFromFile", () => {
  it("uses the theme's name, or the file name without its extension", () => {
    expect(themeFromFile('{"name": " Night "}', "night.json").name).toBe("Night");
    expect(themeFromFile("{}", "My Theme.JSON").name).toBe("My Theme");
  });
});
