import { themes, CUSTOM_THEME_ID } from "../../themes";
import { Modal } from "./Modal";
import { CustomThemeEditor } from "./CustomThemeEditor";
import { CheckIcon } from "./Icons";

interface ThemePickerProps {
  open: boolean;
  onClose: () => void;
  currentThemeId: string;
  onThemeChange: (themeId: string) => void;
  customColors: Record<string, string> | null;
  onCustomColorsChange: (colors: Record<string, string>) => void;
}

export function ThemePicker({
  open,
  onClose,
  currentThemeId,
  onThemeChange,
  customColors,
  onCustomColorsChange,
}: ThemePickerProps) {
  const handleSelect = (themeId: string) => {
    onThemeChange(themeId);
  };

  // Preview swatch for the Custom card: the user's saved colors, or the colors
  // it would seed from (the active preset) when not yet customized.
  const customPreview =
    customColors ?? (themes.find((t) => t.id === currentThemeId)?.colors ?? themes[0].colors);
  const isCustom = currentThemeId === CUSTOM_THEME_ID;

  return (
    <Modal open={open} onClose={onClose} title="Theme" wide>
      <div className="p-6">
        <div className="grid grid-cols-2 gap-3">
          {themes.map((theme) => (
            <button
              key={theme.id}
              onClick={() => handleSelect(theme.id)}
              className={`flex items-center gap-3 p-3 rounded-lg border transition-all cursor-pointer ${
                currentThemeId === theme.id
                  ? "border-accent bg-bg-tertiary"
                  : "border-border hover:border-border-light hover:bg-bg-hover"
              }`}
            >
              {/* Color swatch preview */}
              <div className="flex gap-0.5 flex-shrink-0">
                <div
                  className="w-4 h-8 rounded-l-md"
                  style={{ backgroundColor: theme.colors["bg-secondary"] }}
                />
                <div className="flex flex-col gap-0.5">
                  <div
                    className="w-4 h-[15px]"
                    style={{ backgroundColor: theme.colors["accent"] }}
                  />
                  <div
                    className="w-4 h-[15px] rounded-br-md"
                    style={{ backgroundColor: theme.colors["bg-tertiary"] }}
                  />
                </div>
              </div>

              <div className="text-left min-w-0">
                <p className="text-sm font-medium text-text-primary">{theme.name}</p>
                <div className="flex gap-1 mt-1">
                  {["accent", "success", "warning", "danger"].map((key) => (
                    <div
                      key={key}
                      className="w-3 h-3 rounded-full"
                      style={{ backgroundColor: theme.colors[key] }}
                    />
                  ))}
                </div>
              </div>

              {currentThemeId === theme.id && (
                <CheckIcon size={14} className="text-accent ml-auto flex-shrink-0" />
              )}
            </button>
          ))}

          {/* Custom, user-configurable theme */}
          <button
            key={CUSTOM_THEME_ID}
            onClick={() => handleSelect(CUSTOM_THEME_ID)}
            className={`flex items-center gap-3 p-3 rounded-lg border transition-all cursor-pointer ${
              isCustom
                ? "border-accent bg-bg-tertiary"
                : "border-border hover:border-border-light hover:bg-bg-hover"
            }`}
          >
            {/* Color swatch preview */}
            <div className="flex gap-0.5 flex-shrink-0">
              <div
                className="w-4 h-8 rounded-l-md"
                style={{ backgroundColor: customPreview["bg-secondary"] }}
              />
              <div className="flex flex-col gap-0.5">
                <div
                  className="w-4 h-[15px]"
                  style={{ backgroundColor: customPreview["accent"] }}
                />
                <div
                  className="w-4 h-[15px] rounded-br-md"
                  style={{ backgroundColor: customPreview["bg-tertiary"] }}
                />
              </div>
            </div>

            <div className="text-left min-w-0">
              <p className="text-sm font-medium text-text-primary">Custom</p>
              <div className="flex gap-1 mt-1">
                {["accent", "success", "warning", "danger"].map((key) => (
                  <div
                    key={key}
                    className="w-3 h-3 rounded-full"
                    style={{ backgroundColor: customPreview[key] }}
                  />
                ))}
              </div>
            </div>

            {isCustom && (
              <CheckIcon size={14} className="text-accent ml-auto flex-shrink-0" />
            )}
          </button>
        </div>

        {isCustom && customColors && (
          <CustomThemeEditor colors={customColors} onChange={onCustomColorsChange} />
        )}
      </div>
    </Modal>
  );
}
