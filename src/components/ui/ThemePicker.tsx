import { ReactNode } from "react";
import {
  themes,
  themeAppearance,
  presetFor,
  systemThemePresets,
  CUSTOM_THEME_ID,
  SYSTEM_THEME_ID,
  Theme,
} from "../../themes";
import { CustomThemeEditor } from "./CustomThemeEditor";
import { CheckIcon } from "./Icons";

interface ThemeSettingsProps {
  currentThemeId: string;
  onThemeChange: (themeId: string) => void;
  customColors: Record<string, string> | null;
  onCustomColorsChange: (colors: Record<string, string>) => void;
}

type Colors = Record<string, string>;

function ThemeSwatch({ colors, sideColors = colors }: { colors: Colors; sideColors?: Colors }) {
  return (
    <div className="flex gap-0.5 flex-shrink-0">
      <div
        className="w-4 h-8 rounded-l-md"
        style={{ backgroundColor: sideColors["bg-secondary"] }}
      />
      <div className="flex flex-col gap-0.5">
        <div className="w-4 h-[15px]" style={{ backgroundColor: colors["accent"] }} />
        <div
          className="w-4 h-[15px] rounded-br-md"
          style={{ backgroundColor: colors["bg-tertiary"] }}
        />
      </div>
    </div>
  );
}

interface ThemeCardProps {
  name: string;
  colors: Colors;
  sideColors?: Colors;
  description?: string;
  selected: boolean;
  onSelect: () => void;
}

function ThemeCard({ name, colors, sideColors, description, selected, onSelect }: ThemeCardProps) {
  return (
    <button
      onClick={onSelect}
      className={`flex items-center gap-3 p-3 rounded-lg border transition-all cursor-pointer ${
        selected
          ? "border-accent bg-bg-tertiary"
          : "border-border hover:border-border-light hover:bg-bg-hover"
      }`}
    >
      <ThemeSwatch colors={colors} sideColors={sideColors} />

      <div className="text-left min-w-0">
        <p className="text-sm font-medium text-text-primary">{name}</p>
        {description ? (
          <p className="text-xs text-text-muted mt-1 truncate">{description}</p>
        ) : (
          <div className="flex gap-1 mt-1">
            {["accent", "success", "warning", "danger"].map((key) => (
              <div
                key={key}
                className="w-3 h-3 rounded-full"
                style={{ backgroundColor: colors[key] }}
              />
            ))}
          </div>
        )}
      </div>

      {selected && <CheckIcon size={14} className="text-accent ml-auto flex-shrink-0" />}
    </button>
  );
}

function ThemeSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <h3 className="text-xs font-medium uppercase tracking-wide text-text-muted mb-2">{title}</h3>
      <div className="grid grid-cols-2 gap-3">{children}</div>
    </section>
  );
}

const lightThemes = themes.filter((t) => themeAppearance(t.colors) === "light");
const darkThemes = themes.filter((t) => themeAppearance(t.colors) === "dark");

export function ThemeSettings({
  currentThemeId,
  onThemeChange,
  customColors,
  onCustomColorsChange,
}: ThemeSettingsProps) {
  // Preview swatch for the Custom card: the user's saved colors, or the colors
  // it would seed from (the active preset) when not yet customized.
  const customPreview = customColors ?? presetFor(currentThemeId).colors;
  const isCustom = currentThemeId === CUSTOM_THEME_ID;

  const presetCard = (theme: Theme) => (
    <ThemeCard
      key={theme.id}
      name={theme.name}
      colors={theme.colors}
      selected={currentThemeId === theme.id}
      onSelect={() => onThemeChange(theme.id)}
    />
  );

  return (
    <div className="p-6 space-y-6">
      <ThemeSection title="Automatic">
        <ThemeCard
          name="System"
          colors={systemThemePresets.dark.colors}
          sideColors={systemThemePresets.light.colors}
          description={`${systemThemePresets.light.name} or ${systemThemePresets.dark.name}, following macOS`}
          selected={currentThemeId === SYSTEM_THEME_ID}
          onSelect={() => onThemeChange(SYSTEM_THEME_ID)}
        />
      </ThemeSection>

      <ThemeSection title="Light">{lightThemes.map(presetCard)}</ThemeSection>

      <ThemeSection title="Dark">{darkThemes.map(presetCard)}</ThemeSection>

      <div>
        <ThemeSection title="Custom">
          <ThemeCard
            name="Custom"
            colors={customPreview}
            selected={isCustom}
            onSelect={() => onThemeChange(CUSTOM_THEME_ID)}
          />
        </ThemeSection>

        {isCustom && customColors && (
          <CustomThemeEditor colors={customColors} onChange={onCustomColorsChange} />
        )}
      </div>
    </div>
  );
}
