import { themeAppearance } from "./themes";

/** The embedded editor's built-in theme each app appearance recolors. */
const EDITOR_BASE_THEMES = {
  dark: "Dark Modern",
  light: "Light Modern",
} as const;

const EDITOR_COLORS_BY_TOKEN: Record<string, string[]> = {
  "bg-primary": [
    "editor.background",
    "editorGutter.background",
    "editorGroupHeader.tabsBackground",
    "tab.activeBackground",
    "tab.unfocusedActiveBackground",
    "breadcrumb.background",
    "panel.background",
    "terminal.background",
    "editorStickyScroll.background",
  ],
  "bg-secondary": [
    "sideBar.background",
    "sideBarSectionHeader.background",
    "activityBar.background",
    "titleBar.activeBackground",
    "titleBar.inactiveBackground",
    "statusBar.background",
    "statusBar.noFolderBackground",
    "tab.inactiveBackground",
    "tab.unfocusedInactiveBackground",
    "editor.lineHighlightBackground",
  ],
  "bg-tertiary": [
    "input.background",
    "dropdown.background",
    "editorWidget.background",
    "editorHoverWidget.background",
    "editorSuggestWidget.background",
    "quickInput.background",
    "menu.background",
    "notifications.background",
    "peekViewEditor.background",
  ],
  "bg-hover": ["list.hoverBackground", "toolbar.hoverBackground", "tab.hoverBackground"],
  "bg-active": [
    "list.activeSelectionBackground",
    "list.inactiveSelectionBackground",
    "quickInputList.focusBackground",
    "menu.selectionBackground",
  ],
  border: [
    "sideBar.border",
    "sideBarSectionHeader.border",
    "activityBar.border",
    "titleBar.border",
    "statusBar.border",
    "panel.border",
    "editorGroup.border",
    "tab.border",
    "editorGroupHeader.tabsBorder",
    "editorWidget.border",
    "menu.border",
  ],
  "border-light": ["input.border", "dropdown.border", "checkbox.border"],
  "text-primary": [
    "foreground",
    "editor.foreground",
    "sideBar.foreground",
    "tab.activeForeground",
    "titleBar.activeForeground",
    "input.foreground",
    "dropdown.foreground",
    "list.activeSelectionForeground",
    "editorLineNumber.activeForeground",
    "activityBar.foreground",
    "menu.foreground",
  ],
  "text-secondary": [
    "descriptionForeground",
    "sideBarTitle.foreground",
    "tab.inactiveForeground",
    "titleBar.inactiveForeground",
    "statusBar.foreground",
    "activityBar.inactiveForeground",
  ],
  "text-muted": [
    "disabledForeground",
    "editorLineNumber.foreground",
    "input.placeholderForeground",
    "editorWhitespace.foreground",
  ],
  accent: [
    "focusBorder",
    "button.background",
    "badge.background",
    "activityBarBadge.background",
    "activityBar.activeBorder",
    "tab.activeBorderTop",
    "progressBar.background",
    "textLink.foreground",
    "editorCursor.foreground",
    "statusBarItem.remoteBackground",
  ],
  "accent-hover": ["button.hoverBackground", "textLink.activeForeground"],
  success: ["gitDecoration.addedResourceForeground", "editorGutter.addedBackground"],
  warning: [
    "editorWarning.foreground",
    "list.warningForeground",
    "gitDecoration.modifiedResourceForeground",
    "editorGutter.modifiedBackground",
  ],
  danger: [
    "errorForeground",
    "editorError.foreground",
    "list.errorForeground",
    "inputValidation.errorBorder",
    "gitDecoration.deletedResourceForeground",
    "editorGutter.deletedBackground",
  ],
};

const TRANSLUCENT_EDITOR_COLORS: { key: string; token: string; alpha: string }[] = [
  { key: "editor.selectionBackground", token: "accent", alpha: "4d" },
  { key: "editor.inactiveSelectionBackground", token: "accent", alpha: "26" },
  { key: "editor.findMatchHighlightBackground", token: "warning", alpha: "40" },
];

const ON_ACCENT_FOREGROUNDS = [
  "button.foreground",
  "badge.foreground",
  "activityBarBadge.foreground",
  "statusBarItem.remoteForeground",
];

const OPAQUE_HEX = /^#[0-9a-f]{6}$/i;

function editorColors(colors: Record<string, string>): Record<string, string> {
  const result: Record<string, string> = {};
  for (const [token, keys] of Object.entries(EDITOR_COLORS_BY_TOKEN)) {
    const value = colors[token];
    if (!value) continue;
    for (const key of keys) result[key] = value;
  }
  for (const { key, token, alpha } of TRANSLUCENT_EDITOR_COLORS) {
    const value = colors[token];
    if (value && OPAQUE_HEX.test(value)) result[key] = `${value}${alpha}`;
  }
  if (colors.accent) for (const key of ON_ACCENT_FOREGROUNDS) result[key] = "#ffffff";
  return result;
}

/**
 * Default settings that recolor the editor's built-in theme with the app's colors. They sit
 * below user and workspace settings, so a theme the user picks in the editor wins.
 */
export function editorConfigurationDefaults(colors: Record<string, string>) {
  const baseTheme = EDITOR_BASE_THEMES[themeAppearance(colors)];
  return {
    "workbench.colorTheme": baseTheme,
    "workbench.colorCustomizations": { [`[${baseTheme}]`]: editorColors(colors) },
  };
}
