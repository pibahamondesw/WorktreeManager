import { themes } from "../../themes";
import { Modal } from "./Modal";
import { CheckIcon } from "./Icons";

interface ThemePickerProps {
  open: boolean;
  onClose: () => void;
  currentThemeId: string;
  onThemeChange: (themeId: string) => void;
}

export function ThemePicker({ open, onClose, currentThemeId, onThemeChange }: ThemePickerProps) {
  const handleSelect = (themeId: string) => {
    onThemeChange(themeId);
  };

  return (
    <Modal open={open} onClose={onClose} title="Theme">
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
        </div>
      </div>
    </Modal>
  );
}
