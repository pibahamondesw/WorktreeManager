import { describe, expect, it } from "vitest";
import { editorConfigurationDefaults } from "./editorTheme";
import { themes } from "./themes";

const preset = (id: string) => themes.find((theme) => theme.id === id)!.colors;

describe("editor configuration defaults", () => {
  it("recolors the dark built-in theme for dark app themes, only while it is selected", () => {
    const colors = preset("default");
    const defaults = editorConfigurationDefaults(colors);
    expect(defaults["workbench.colorTheme"]).toBe("Dark Modern");
    const customizations = defaults["workbench.colorCustomizations"]["[Dark Modern]"];
    expect(Object.keys(defaults["workbench.colorCustomizations"])).toEqual(["[Dark Modern]"]);
    expect(customizations["editor.background"]).toBe(colors["bg-primary"]);
    expect(customizations["sideBar.background"]).toBe(colors["bg-secondary"]);
    expect(customizations["focusBorder"]).toBe(colors.accent);
    expect(customizations["editor.selectionBackground"]).toBe(`${colors.accent}4d`);
  });

  it("recolors the light built-in theme for light app themes", () => {
    const defaults = editorConfigurationDefaults(preset("solarized-light"));
    expect(defaults["workbench.colorTheme"]).toBe("Light Modern");
    expect(defaults["workbench.colorCustomizations"]["[Light Modern]"]["editor.background"]).toBe(
      "#fdf6e3"
    );
  });

  it("maps custom colors and leaves out what they do not define", () => {
    const defaults = editorConfigurationDefaults({ "bg-primary": "#101010", accent: "#abc" });
    const customizations = defaults["workbench.colorCustomizations"]["[Dark Modern]"];
    expect(customizations["editor.background"]).toBe("#101010");
    expect(customizations["button.background"]).toBe("#abc");
    expect(customizations["button.foreground"]).toBe("#ffffff");
    expect(customizations).not.toHaveProperty("editor.selectionBackground");
    expect(customizations).not.toHaveProperty("foreground");
  });
});
